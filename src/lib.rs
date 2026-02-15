pub use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;

pub use anyhow::{self, Context};
pub use cargo_toml::Manifest;
use serde::Deserialize;
use sha2::{Digest, Sha256};

static FORMULA_TMPL: &str = include_str!("formula.rb");
static GH_FORMULA_TMPL: &str = include_str!("gh_strategy.rb");

/// Metadata from a `formulaic.toml` file, for non-Cargo projects.
#[derive(Debug, Clone, Deserialize)]
pub struct GenericManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
}

impl GenericManifest {
    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
        let manifest: Self =
            toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(manifest)
    }

    /// Extract (owner, repo) from the repository URL.
    pub fn owner_repo(&self) -> anyhow::Result<(String, String)> {
        let repository = self
            .repository
            .as_deref()
            .with_context(|| "formulaic.toml must have a repository field")?;
        parse_owner_repo(repository)
    }
}

/// Parse a GitHub repository URL into (owner, repo).
pub fn parse_owner_repo(repository: &str) -> anyhow::Result<(String, String)> {
    let mut chunks: Vec<&str> = repository.split('/').collect();
    let repo = chunks
        .pop()
        .with_context(|| "Invalid repository URL format")?
        .trim_end_matches(".git");
    let owner = chunks.pop().with_context(|| "Invalid repository URL format")?;
    Ok((owner.to_string(), repo.to_string()))
}

#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub name: String,
    pub package_name: String,
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub cpu: String,
    pub os: String,
    pub digest: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct FormulaContext {
    pub package: String,
    pub description: String,
    pub executable: String,
    pub homepage: String,
    pub version: String,
    pub license: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug)]
pub struct AssetMatcher {
    pub target_mappings: HashMap<&'static str, (&'static str, &'static str)>,
}

impl Default for AssetMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetMatcher {
    pub fn new() -> Self {
        let mut mappings = HashMap::new();
        // Homebrew-specific platform mappings
        mappings.insert("aarch64-apple-darwin", ("mac", "arm"));
        mappings.insert("x86_64-apple-darwin", ("mac", "intel"));
        mappings.insert("x86_64-unknown-linux-gnu", ("linux", "intel"));
        mappings.insert("aarch64-unknown-linux-gnu", ("linux", "arm"));

        Self {
            target_mappings: mappings,
        }
    }

    pub fn extract_platform(&self, asset_name: &str) -> Option<(&'static str, &'static str)> {
        for (target, (os, cpu)) in &self.target_mappings {
            if asset_name.contains(target) {
                return Some((*os, *cpu));
            }
        }
        None
    }

    /// Check if an asset name refers to a universal (fat) macOS binary.
    pub fn is_universal(&self, asset_name: &str) -> bool {
        asset_name.contains("universal-apple-darwin")
    }

    /// Extract all platforms an asset supports. Universal macOS binaries
    /// expand to both arm and intel entries.
    pub fn extract_platforms(&self, asset_name: &str) -> Vec<(&'static str, &'static str)> {
        if self.is_universal(asset_name) {
            return vec![("mac", "arm"), ("mac", "intel")];
        }
        self.extract_platform(asset_name).into_iter().collect()
    }

    /// Check whether the portion of the filename after the binary name prefix
    /// matches a known target triple or a universal binary pattern.
    pub fn matches_target(&self, after_prefix: &str) -> bool {
        after_prefix.starts_with("universal-apple-darwin")
            || self
                .target_mappings
                .keys()
                .any(|target| after_prefix.starts_with(target))
    }
}

pub fn get_binaries_from_manifest(manifest: &Manifest, target_bin: Option<&str>) -> anyhow::Result<Vec<BinaryInfo>> {
    let Some(ref package) = manifest.package else {
        anyhow::bail!("The Rust project must have at least one package in it.");
    };

    if manifest.bin.is_empty() {
        anyhow::bail!("No support for making formulas for Rust libraries, only for Rust binaries.");
    }

    let mut binaries = Vec::new();

    for bin_product in &manifest.bin {
        let Some(ref executable) = bin_product.name else {
            continue; // Skip binaries without names
        };

        // Filter by target binary if specified
        if let Some(target) = target_bin
            && executable != target
        {
            continue;
        }

        binaries.push(BinaryInfo {
            name: executable.clone(),
            package_name: package.name().to_string(),
        });
    }

    if binaries.is_empty() {
        if let Some(target) = target_bin {
            anyhow::bail!("Binary '{}' not found in manifest", target);
        } else {
            anyhow::bail!("No valid binaries found in manifest");
        }
    }

    Ok(binaries)
}

pub fn create_base_context(manifest: &Manifest, binary: &BinaryInfo) -> anyhow::Result<FormulaContext> {
    use heck::ToUpperCamelCase;

    let Some(ref package) = manifest.package else {
        anyhow::bail!("The Rust project must have at least one package in it.");
    };

    let homepage = package.homepage().map_or(String::default(), |xs| xs.to_owned());
    let description = package.description().map_or(String::default(), |xs| xs.to_owned());
    let license = package.license().map_or("unlicensed".to_string(), |xs| xs.to_owned());

    Ok(FormulaContext {
        package: binary.name.to_upper_camel_case(),
        description,
        executable: binary.name.clone(),
        homepage,
        version: package.version().to_string(),
        license,
        assets: Vec::new(), // we'll populate this later
    })
}

pub fn create_base_context_from_generic(manifest: &GenericManifest) -> FormulaContext {
    use heck::ToUpperCamelCase;

    FormulaContext {
        package: manifest.name.to_upper_camel_case(),
        description: manifest.description.clone().unwrap_or_default(),
        executable: manifest.name.clone(),
        homepage: manifest.homepage.clone().unwrap_or_default(),
        version: manifest.version.clone(),
        license: manifest
            .license
            .clone()
            .unwrap_or_else(|| "unlicensed".to_string()),
        assets: Vec::new(),
    }
}

impl Asset {
    pub fn from_release_asset(v: &roctogen::models::ReleaseAsset, matcher: &AssetMatcher) -> anyhow::Result<Self> {
        let filename = v
            .name
            .as_ref()
            .with_context(|| format!("asset {:?} has an empty name", v.id))?;

        if !filename.ends_with(".tar.gz") {
            anyhow::bail!("asset {:?} is not a tarball", v.id);
        }

        let url = v
            .browser_download_url
            .as_ref()
            .with_context(|| format!("asset {:?} doesn't have a download url", v.id))?;

        let digest = if let Some(ref digest) = v.digest {
            digest.split_once(':').map(|split| split.1.to_owned())
        } else {
            None
        };

        let digest = if let Some(d) = digest {
            d
        } else {
            find_digest(filename.as_str(), url.as_str())
                .with_context(|| format!("Cannot calculate digest for asset {filename}"))?
        };

        let (os, cpu) = matcher
            .extract_platform(filename)
            .with_context(|| format!("Cannot determine platform for asset {filename}"))?;

        Ok(Self {
            cpu: cpu.to_string(),
            os: os.to_string(),
            digest,
            url: url.to_owned(),
        })
    }

    /// Create assets from a release asset that may be a universal binary.
    /// Universal binaries produce two assets (arm + intel); others produce one.
    pub fn assets_from_release_asset(
        v: &roctogen::models::ReleaseAsset,
        matcher: &AssetMatcher,
    ) -> anyhow::Result<Vec<Self>> {
        let filename = v
            .name
            .as_ref()
            .with_context(|| format!("asset {:?} has an empty name", v.id))?;

        if !filename.ends_with(".tar.gz") {
            anyhow::bail!("asset {:?} is not a tarball", v.id);
        }

        let url = v
            .browser_download_url
            .as_ref()
            .with_context(|| format!("asset {:?} doesn't have a download url", v.id))?;

        let digest = if let Some(ref digest) = v.digest {
            digest.split_once(':').map(|split| split.1.to_owned())
        } else {
            None
        };

        let digest = if let Some(d) = digest {
            d
        } else {
            find_digest(filename.as_str(), url.as_str())
                .with_context(|| format!("Cannot calculate digest for asset {filename}"))?
        };

        let platforms = matcher.extract_platforms(filename);
        if platforms.is_empty() {
            anyhow::bail!("Cannot determine platform for asset {filename}");
        }

        Ok(platforms
            .into_iter()
            .map(|(os, cpu)| Self {
                cpu: cpu.to_string(),
                os: os.to_string(),
                digest: digest.clone(),
                url: url.to_owned(),
            })
            .collect())
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

pub fn find_digest(filename: &str, url: &str) -> anyhow::Result<String> {
    // look for a local shasum file
    let digestpath = format!("{filename}.sha256");
    if let Ok(exists) = std::fs::exists(&digestpath)
        && exists
        && let Ok(mut fp) = std::fs::File::open(&digestpath)
    {
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

pub fn render_to_string(
    use_gh: bool,
    custom_template: Option<&str>,
    context: &FormulaContext,
) -> anyhow::Result<String> {
    let mut engine = upon::Engine::new();
    if let Some(tmpl) = custom_template {
        engine.add_template("formula", tmpl)?;
    } else if use_gh {
        engine.add_template("formula", GH_FORMULA_TMPL)?;
    } else {
        engine.add_template("formula", FORMULA_TMPL)?;
    }

    // Convert FormulaContext to upon::Value
    let mut map: BTreeMap<&str, upon::Value> = BTreeMap::new();
    map.insert("package", context.package.clone().into());
    map.insert("description", context.description.clone().into());
    map.insert("executable", context.executable.clone().into());
    map.insert("homepage", context.homepage.clone().into());
    map.insert("version", context.version.clone().into());
    map.insert("license", context.license.clone().into());
    map.insert("assets", context.assets.clone().into());

    let values = upon::to_value(map)?;
    Ok(engine.template("formula").render(&values).to_string()?)
}
