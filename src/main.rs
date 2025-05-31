//! doc string

use std::collections::BTreeMap;
use std::io::{Read, Write};

use cargo_toml::Inheritable::Set;
use cargo_toml::Manifest;
use heck::ToTitleCase;
use roctogen::endpoints::repos;
use roctogen::models::ReleaseAsset;
use roctokit::adapters::client;
use roctokit::auth::Auth;
use sha2::{Digest, Sha256};

static FORMULA_TMPL: &str = include_str!("formula.rb");

fn render_to_string(values: &upon::Value) -> anyhow::Result<String> {
    let mut engine = upon::Engine::new();
    engine.add_template("formula", FORMULA_TMPL)?;
    Ok(engine.template("formula").render(values).to_string()?)
}

#[derive(Debug, Clone)]
struct Asset {
    cpu: String,
    os: String,
    digest: String,
    url: String,
}

impl TryFrom<&ReleaseAsset> for Asset {
    type Error = anyhow::Error;

    fn try_from(v: &ReleaseAsset) -> Result<Self, Self::Error> {
        let Some(ref filename) = v.name else {
            return Err(anyhow::anyhow!("asset {:?} has an empty name", v.id));
        };
        if !filename.ends_with(".tar.gz") {
            return Err(anyhow::anyhow!("asset {:?} is not a tarball", v.id));
        }
        let Some(ref url) = v.browser_download_url else {
            return Err(anyhow::anyhow!("asset {:?} doesn't have a download url somehow", v.id));
        };

        let digest = if let Some(ref digest) = v.digest {
            digest.split_once(':').map(|split| split.1.to_owned())
        } else {
            None
        };
        let digest = if let Some(d) = digest {
            d
        } else if let Ok(d) = find_digest(filename.as_str(), url.as_str()) {
            d
        } else {
            return Err(anyhow::anyhow!(
                "Skipping asset {filename} because we cannot calculate a digest for it."
            ));
        };

        let os = if url.contains("apple") || url.contains("mac") || url.contains("darwin") {
            "mac".to_string()
        } else if url.contains("linux") {
            "linux".to_string()
        } else {
            "unknown".to_string()
        };

        let cpu = if url.contains("intel") || url.contains("x86_64") {
            "intel".to_string()
        } else if url.contains("aarch") || url.contains("arm") {
            "arm".to_string()
        } else {
            "unknown".to_string()
        };

        Ok(Self {
            cpu,
            os,
            digest,
            url: url.to_owned(),
        })
    }
}

impl From<Asset> for upon::Value {
    fn from(v: Asset) -> Self {
        let mut result: BTreeMap<String, String> = BTreeMap::new();
        result.insert("cpu".to_string(), v.cpu);
        result.insert("os".to_string(), v.os);
        result.insert("sha256".to_string(), v.digest);
        result.insert("url".to_string(), v.url);
        result.into()
    }
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
            "Can't guess the repository if neither running in an action nor given the info from Cargo.toml."
        ));
    };

    let mut chunks: Vec<&str> = repository.split('/').collect();
    // yeah, these are unwraps of options
    let repo = chunks.pop().unwrap_or_default().trim_end_matches(".git").to_string();
    let owner = chunks.pop().unwrap_or_default().to_string();

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
    map.insert("executable", executable.to_string().into());

    let license = if let Some(license) = package.license() {
        Some(license.to_string())
    } else if let Some(v) = repo_info.license {
        v.name.clone()
    } else {
        None
    };
    if let Some(ref lic) = license {
        map.insert("license", lic.to_owned().into());
    } else {
        map.insert("license", "unlicensed".into());
    }

    let version = package.version();
    map.insert("version", version.into());

    let mut asset_list: Vec<_> = Vec::new();
    if let Some(ref assets) = latest_release.assets {
        for asset in assets {
            let Ok(mapped) = Asset::try_from(asset) else {
                continue;
            };
            asset_list.push(mapped);
        }
    }
    map.insert("assets", asset_list.into());

    let values = upon::to_value(map)?;
    let rendered = render_to_string(&values)?;
    // This rendered file needs to be commited to the homebrew tap repo
    // as `Formula/executable.rb`
    let formula_path = format!("{executable}.rb");
    let mut fp = std::fs::File::create(&formula_path)?;
    let count = fp.write(rendered.as_bytes())?;
    if count == 0 {
        eprintln!("zero-length formula file indicates trouble in River City.")
    }
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
    if let Ok(mut fp) = std::fs::File::open(filename) {
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
    let args: Vec<String> = std::env::args().rev().collect();
    let manifest_path = if let Some(argument) = args.first() {
        argument.to_owned()
    } else {
        "./Cargo.toml".to_string()
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
            executable: "frobber",
            assets: [{
                    os: "mac", cpu: "arm", url: "https://example.com/", sha256: "deadbeef"
                }, {
                    os: "linux", cpu: "intel", url: "https://example.com/", sha256: "cafed00d"
                }
            ]
        };
        let rendered = render_to_string(&manifest).expect("rendering the template failed");
        eprintln!("{rendered}");
        assert!(rendered.contains("sha256 \"cafed00d\""));
        assert!(rendered.contains("bin.install \"frobber\" if OS.mac?"));
    }
}
