//! doc string

use std::collections::BTreeMap;
use std::io::{Read, Write};

use cargo_toml::Inheritable::Set;
use cargo_toml::Manifest;
use heck::ToTitleCase;
use roctogen::endpoints::repos;
use roctokit::adapters::client;
use roctokit::auth::Auth;
use sha2::{Digest, Sha256};

static FORMULA_TMPL: &str = include_str!("formula.rb");

fn render_to_string(values: &upon::Value) -> anyhow::Result<String> {
    let mut engine = upon::Engine::new();
    engine.add_template("formula", FORMULA_TMPL)?;
    Ok(engine.template("formula").render(values).to_string()?)
}

fn render_formula(manifest: &Manifest, github: &roctokit::adapters::ureq::Client) -> anyhow::Result<String> {
    // get a package to use as info source
    let Some(ref package) = manifest.package else {
        return Err(anyhow::anyhow!(
            "The Rust project must have at least one package in it."
        ));
    };

    // We are going to take as our executable target the first bin in the bins list.
    let Some(bin_product) = manifest.bin.first() else {
        return Err(anyhow::anyhow!(
            "No support for making formulas for Rust libraries, only for Rust binaries."
        ));
    };
    let Some(ref executable) = bin_product.name else {
        return Err(anyhow::anyhow!("The binary executable needs a name."));
    };

    // TODO look for repo info in environment if in CI
    let Some(Set(ref repository)) = package.repository else {
        return Err(anyhow::anyhow!(
            "We can't guess which repository we need info for if we are neither running in an action nor given the info from Cargo.toml."
        ));
    };

    let mut chunks: Vec<&str> = repository.split('/').into_iter().collect();
    // yeah, these are unwraps of options
    let repo = chunks.pop().unwrap().trim_end_matches(".git").to_string();
    let owner = chunks.pop().unwrap().to_string();

    // gather release information
    let repo_api = repos::new(github);
    let repo_info = repo_api
        .get(owner.as_str(), repo.as_str())
        .expect("unable to fetch repo info");
    let latest_release = repo_api
        .get_latest_release(owner.as_str(), repo.as_str())
        .expect("unable to get latest release");

    let mut map: BTreeMap<&str, upon::Value> = BTreeMap::new();
    map.insert("package", package.name().to_title_case().into());
    map.insert("description", repo_info.description.unwrap_or_default().clone().into());
    map.insert("homepage", repo_info.url.unwrap_or_default().clone().into());

    let license = if let Some(license) = package.license() {
        license.to_string()
    } else {
        if let Some(v) = repo_info.license {
            v.name.unwrap_or_else(|| "who knows".to_string())
        } else {
            "beats me".to_string()
        }
    };
    map.insert("license", license.into());

    let version = package.version();
    map.insert("version", version.into());
    let mut asset_list: Vec<_> = Vec::new();

    if let Some(ref assets) = latest_release.assets {
        for asset in assets {
            let Some(ref filename) = asset.name else {
                // eprintln!("skipping asset with empty name");
                continue;
            };
            if !filename.ends_with(".tar.gz") {
                // eprintln!("skipping non-tarball asset");
                continue;
            }
            let Some(ref url) = asset.browser_download_url else {
                // eprintln!("skipping asset without a download url");
                continue;
            };
            let mut asset_map: BTreeMap<&str, String> = BTreeMap::new();
            // asset_map.insert("filename", filename.clone());
            asset_map.insert("url", url.to_owned());

            let digest = if let Some(ref digest) = asset.digest {
                if let Some(split) = digest.split_once(':') {
                    Some(split.1.to_owned())
                } else {
                    None
                }
            } else {
                None
            };
            let digest = if let Some(d) = digest {
                d
            } else if let Ok(d) = find_digest(filename.as_str(), url.as_str()) {
                d
            } else {
                // eprintln!("Skipping asset {filename} because we cannot calculate a digest for it.");
                continue;
            };
            asset_map.insert("sha256", digest);

            if url.contains("apple") || url.contains("mac") || url.contains("darwin") {
                asset_map.insert("os", "mac".to_string());
            } else if url.contains("linux") {
                asset_map.insert("os", "linux".to_string());
            } else {
                asset_map.insert("os", "unknown".to_string());
            }

            if url.contains("intel") || url.contains("x86_64") {
                asset_map.insert("cpu", "intel".to_string());
            } else if url.contains("aarch") || url.contains("arm") {
                asset_map.insert("cpu", "arm".to_string());
            } else {
                asset_map.insert("cpu", "unknown".to_string());
            }

            asset_map.insert("executable", executable.clone());
            asset_list.push(asset_map);
        }
    }
    map.insert("assets", asset_list.into());

    let values = upon::to_value(map)?;
    let rendered = render_to_string(&values)?;
    // eprintln!("{rendered}");
    // This rendered file needs to be commited to the homebrew tap repo
    // as `Formula/executable.rb`
    let formula_path = format!("{executable}.rb");
    let mut fp = std::fs::File::create(&formula_path)?;
    fp.write(rendered.as_bytes())?;
    Ok(formula_path)
}

fn find_digest(filename: &str, url: &str) -> anyhow::Result<String> {
    // look for a local shasum file
    let digestpath = format!("{filename}.sha256");
    if let Ok(exists) = std::fs::exists(&digestpath) {
        if exists {
            if let Ok(mut fp) = std::fs::File::open(&digestpath) {
                let mut digest = String::new();
                if let Ok(_length) = fp.read_to_string(&mut digest) {
                    return Ok(digest);
                }
            }
        }
    }

    // try a local tarball
    if let Ok(mut fp) = std::fs::File::open(&filename) {
        let mut buffer: Vec<u8> = Vec::new();
        if let Ok(_length) = fp.read(&mut buffer) {
            let digest = Sha256::digest(&buffer);
            return Ok(hex::encode(digest));
        }
    }

    // well, we have to do this the hard way. Here we start
    // returning our errors, because this is our last try.
    let mut response = ureq::get(url).call()?;
    let payload: Vec<u8> = response.body_mut().read_to_vec()?;
    let digest = Sha256::digest(&payload);
    Ok(hex::encode(digest))
}

fn main() -> anyhow::Result<()> {
    let token = std::env::var("GITHUB_ACCESS_TOKEN").expect("unable to find GITHUB_ACCESS_TOKEN");

    // Read the desired manifest or fail fast.
    let args: Vec<String> = std::env::args().into_iter().rev().collect();
    let manifest_path = if let Some(argument) = args.first() {
        argument.to_owned()
    } else {
        "./".to_string()
    };
    let manifest = cargo_toml::Manifest::from_path(manifest_path)?;

    let auth = Auth::Token(token);
    let github = client(&auth)?;

    let formula_path = render_formula(&manifest, &github)?;
    println!("{formula_path}");

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn render_template() {
        let manifest = upon::value! {
            package: "Frobber",
            description: "Frobs the whizzbanger",
            version: "1.0.5",
            license: "MIT",
            homepage: "https://example.com",
            assets: [{
                    os: "mac", cpu: "arm", executable: "frobber", url: "https://example.com/", sha256: "deadbeef"
                }, {
                    os: "linux", cpu: "intel", executable: "frobber", url: "https://example.com/", sha256: "cafed00d"
                }
            ]
        };
        let rendered = render_to_string(&manifest).expect("rendering the template failed");
        // eprintln!("{rendered}");
        assert!(rendered.contains("sha256 \"cafed00d\""));
    }
}
