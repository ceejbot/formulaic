//! This is a typical Rust clap derive-style cli app.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use cargo_toml::Manifest;
use clap::Parser;
use clap::builder::Styles;
use clap::builder::styling::AnsiColor;
use heck::ToTitleCase;
use roctogen::endpoints::repos;
use roctogen::models::ReleaseAsset;
use roctokit::adapters::client;
use roctokit::auth::Auth;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Parser)]
#[clap(author, version, styles = v3_styles())]
/// Generates a homebrew formula file for first bin mentioned in the given crate manifest.
///
/// Requires a valid github token in GITHUB_ACCESS_TOKEN or GITHUB_TOKEN.
struct Args {
    /// path to the Cargo.toml file for the installable binary
    #[arg(default_value = "./Cargo.toml")]
    manifest: String,
    /// Use the `gh` cli download strategy; useful for private tap repos.
    #[arg(long = "gh-cli-strategy", short = 'g', default_value_t = false, global = true)]
    use_gh_strategy: bool,
    /// If you have no repo-reading API permissions, we'll use only local data.
    #[arg(long = "no-perms", short = 'n', default_value_t = false, global = true)]
    no_perms: bool,
}

fn v3_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Green.on_default())
}

static FORMULA_TMPL: &str = include_str!("formula.rb");
static GH_FORMULA_TMPL: &str = include_str!("gh_strategy.rb");

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

        let os = extract_os(url);
        let cpu = extract_cpu(url);

        Ok(Self {
            cpu,
            os,
            digest,
            url: url.to_owned(),
        })
    }
}

fn extract_os(input: &str) -> String {
    if input.contains("apple") || input.contains("mac") || input.contains("darwin") {
        "mac".to_string()
    } else if input.contains("linux") {
        "linux".to_string()
    } else {
        "unknown".to_string()
    }
}

fn extract_cpu(input: &str) -> String {
    if input.contains("intel") || input.contains("x86_64") {
        "intel".to_string()
    } else if input.contains("aarch") || input.contains("arm") {
        "arm".to_string()
    } else {
        "unknown".to_string()
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

fn find_digest(filename: &str, url: &str) -> anyhow::Result<String> {
    // look for a local shasum file
    let digestpath = format!("{filename}.sha256");
    if let Ok(exists) = std::fs::exists(&digestpath) {
        if exists {
            if let Ok(mut fp) = std::fs::File::open(&digestpath) {
                let mut digest = String::new();
                if let Ok(length) = fp.read_to_string(&mut digest) {
                    // We need to split off any non-digest junk.
                    // the digest itself is exactly 64 char long
                    if length == 64 {
                        return Ok(digest);
                    }
                    if length > 64 {
                        if let Some(slice) = digest.strip_prefix("sha256:") {
                            return Ok(slice.to_string());
                        }
                        let ending = format!("  {filename}");
                        if let Some(slice) = digest.strip_suffix(ending.as_str()) {
                            return Ok(slice.to_string());
                        }
                        if let Some(loc) = digest.rfind(" = ") {
                            let (_first, digest) = digest.split_at(loc + 3);
                            return Ok(digest.trim().to_string());
                        }
                    }
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

fn make_context_common(manifest: &Manifest) -> anyhow::Result<(BTreeMap<&str, upon::Value>, String)> {
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

    let homepage = package.homepage().map_or(String::default(), |xs| xs.to_owned());
    let description = package.description().map_or(String::default(), |xs| xs.to_owned());

    let mut map: BTreeMap<&str, upon::Value> = BTreeMap::new();
    map.insert("package", package.name().to_title_case().into());
    map.insert("description", description.into());
    map.insert("executable", executable.to_string().into());
    map.insert("homepage", homepage.into());
    map.insert("version", package.version().into());

    if let Some(ref lic) = package.license() {
        map.insert("license", lic.to_owned().into());
    } else {
        map.insert("license", "unlicensed".into());
    }

    Ok((map, executable.into()))
}

fn make_context(
    manifest: &Manifest,
    github: &roctokit::adapters::ureq::Client,
) -> anyhow::Result<(upon::Value, String)> {
    let (mut map, executable) = make_context_common(manifest)?;
    let Some(ref package) = manifest.package else {
        return Err(anyhow::anyhow!(
            "The Rust project must have at least one package in it."
        ));
    };

    let repository = package.repository().map_or(String::default(), |xs| xs.to_owned());
    let mut chunks: Vec<&str> = repository.split('/').collect();
    let repo = chunks.pop().unwrap_or_default().trim_end_matches(".git").to_string();
    let owner = chunks.pop().unwrap_or_default().to_string();

    // gather release information
    let repo_api = repos::new(github);
    let latest_release = repo_api
        .get_latest_release(owner.as_str(), repo.as_str())
        .expect("unable to get latest release");
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
    Ok((values, executable.to_string()))
}

fn make_context_local(manifest: &Manifest, manifest_path: &str) -> anyhow::Result<(upon::Value, String)> {
    let (mut map, executable) = make_context_common(manifest)?;
    let Some(ref package) = manifest.package else {
        return Err(anyhow::anyhow!(
            "The Rust project must have at least one package in it."
        ));
    };

    let version = package.version().to_string();
    let repository = package.repository().map_or(String::default(), |xs| xs.to_owned());
    let mut chunks: Vec<&str> = repository.split('/').collect();
    let repo = chunks.pop().unwrap_or_default().trim_end_matches(".git").to_string();
    let owner = chunks.pop().unwrap_or_default().to_string();

    // gh release view -R vlognow/codefact --json assets
    // { "assets": Vec<ReleaseAsset> }

    // Iterate on .tar.gz files in a a dist/ dir at the same level as Cargo.toml
    let mut asset_list: Vec<_> = Vec::new();
    let mut dir = PathBuf::new();
    dir.push(manifest_path);
    dir.pop();
    dir.push("dist");
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let Ok(entry) = entry else {
                continue;
            };
            let fullpath = entry.path();
            let Some(basename) = fullpath.as_path().file_name() else {
                continue;
            };
            let Some(ext) = fullpath.as_path().extension() else {
                continue;
            };

            if !fullpath.is_dir() && ext.to_ascii_lowercase().eq("gz") {
                let path_str = format!("{}", fullpath.display());
                // https://github.com/vlognow/codefact/releases/download/v1.0.4/codefact-aarch64-apple-darwin.tar.gz
                let url = format!(
                    "https://github.com/{owner}/{repo}/releases/download/v{version}/{}",
                    basename.display()
                );
                let Ok(digest) = find_digest(path_str.as_str(), &url) else {
                    continue;
                };
                let os = extract_os(url.as_str());
                let cpu = extract_cpu(url.as_str());

                let asset = Asset { cpu, os, digest, url };
                asset_list.push(asset);
            }
        }
    }

    map.insert("assets", asset_list.into());
    let values = upon::to_value(map)?;
    Ok((values, executable.to_string()))
}

fn render_to_string(use_gh: bool, values: &upon::Value) -> anyhow::Result<String> {
    let mut engine = upon::Engine::new();
    if use_gh {
        engine.add_template("formula", GH_FORMULA_TMPL)?;
    } else {
        engine.add_template("formula", FORMULA_TMPL)?;
    }
    Ok(engine.template("formula").render(values).to_string()?)
}

fn render_formula(use_gh: bool, executable: String, context: &upon::Value) -> anyhow::Result<String> {
    let rendered = render_to_string(use_gh, context)?;
    let formula_path = format!("{executable}.rb");
    let mut fp = std::fs::File::create(&formula_path)?;
    let count = fp.write(rendered.as_bytes())?;
    if count == 0 {
        eprintln!("zero-length formula file indicates trouble in River City.")
    }
    Ok(formula_path)
}

/// Parse arguments and act.
fn main() -> anyhow::Result<()> {
    let token = if let Ok(t) = std::env::var("GITHUB_ACCESS_TOKEN") {
        t
    } else if let Ok(t) = std::env::var("GITHUB_TOKEN") {
        t
    } else {
        return Err(anyhow::anyhow!(
            "unable to find a token in either GITHUB_ACCESS_TOKEN or GITHUB_TOKEN"
        ));
    };

    let args = Args::parse();
    let manifest = cargo_toml::Manifest::from_path(&args.manifest)?;

    let auth = Auth::Token(token);
    let github = client(&auth)?;

    let (context, executable) = if args.no_perms {
        make_context_local(&manifest, args.manifest.as_str())?
    } else {
        make_context(&manifest, &github)?
    };
    let formula_path = render_formula(args.use_gh_strategy, executable, &context)?;
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
        let rendered = render_to_string(false, &manifest).expect("rendering the template failed");
        eprintln!("{rendered}");
        assert!(rendered.contains("sha256 \"cafed00d\""));
        assert!(rendered.contains("bin.install \"frobber\" if OS.mac?"));
    }
}
