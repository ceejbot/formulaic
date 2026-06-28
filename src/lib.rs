use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

pub use anyhow::{self, Context};
pub use cargo_toml::Manifest;
use serde::Deserialize;
use sha2::{Digest, Sha256};

static FORMULA_TMPL: &str = include_str!("formula.rb");
/// The gh-cli download-strategy class, prepended to the formula when the
/// `--gh-cli-strategy` flag (or `gh-cli-strategy = true`) is in effect.
static GH_DOWNLOAD_STRATEGY: &str = include_str!("gh_download_strategy.rb");

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
    #[serde(default, rename = "gh-cli-strategy")]
    pub use_gh_strategy: Option<bool>,
    #[serde(default)]
    pub bins: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub caveats: Option<String>,
}

impl GenericManifest {
    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
        let manifest: Self = toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(manifest)
    }

    /// Extract (owner, repo) from the repository URL.
    pub fn owner_repo(&self) -> anyhow::Result<(String, String)> {
        let repository = self
            .repository
            .as_deref()
            .context("formulaic.toml must have a repository field")?;
        parse_owner_repo(repository)
    }
}

/// Parse a GitHub repository URL into (owner, repo).
pub fn parse_owner_repo(repository: &str) -> anyhow::Result<(String, String)> {
    let mut chunks: Vec<&str> = repository.split('/').collect();
    let repo = chunks
        .pop()
        .context("Invalid repository URL format")?
        .trim_end_matches(".git");
    let owner = chunks.pop().with_context(|| "Invalid repository URL format")?;
    Ok((owner.to_string(), repo.to_string()))
}

/// A binary target to generate a formula for, paired with its owning package.
#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub name: String,
    pub package_name: String,
}

/// A single download entry in a formula: one `(os, cpu)` target with the URL
/// to fetch and the SHA-256 digest to verify it against.
#[derive(Debug, Clone)]
pub struct Asset {
    pub cpu: String,
    pub os: String,
    pub digest: String,
    pub url: String,
}

/// Everything the formula template needs to render a single formula.
#[derive(Debug, Clone)]
pub struct FormulaContext {
    pub package: String,
    pub description: String,
    pub executable: String,
    pub executables: Vec<String>,
    pub homepage: String,
    pub version: String,
    pub license: String,
    pub caveats: Option<String>,
    pub assets: Vec<Asset>,
}

/// Homebrew-specific platform mappings: (target triple, os, cpu).
const TARGET_MAPPINGS: &[(&str, &str, &str)] = &[
    ("aarch64-apple-darwin", "mac", "arm"),
    ("x86_64-apple-darwin", "mac", "intel"),
    ("x86_64-unknown-linux-gnu", "linux", "intel"),
    ("aarch64-unknown-linux-gnu", "linux", "arm"),
];

/// Map an asset filename to its Homebrew `(os, cpu)` target, if the filename
/// contains a known target triple.
pub fn extract_platform(asset_name: &str) -> Option<(&'static str, &'static str)> {
    TARGET_MAPPINGS
        .iter()
        .find(|(target, _, _)| asset_name.contains(target))
        .map(|(_, os, cpu)| (*os, *cpu))
}

/// Check if an asset name refers to a universal (fat) macOS binary.
pub fn is_universal(asset_name: &str) -> bool {
    asset_name.contains("universal-apple-darwin")
}

/// Every Homebrew target an asset supports. Universal macOS binaries expand to
/// both arm and intel entries.
pub fn extract_platforms(asset_name: &str) -> Vec<(&'static str, &'static str)> {
    if is_universal(asset_name) {
        return vec![("mac", "arm"), ("mac", "intel")];
    }
    extract_platform(asset_name).into_iter().collect()
}

/// Check whether the portion of a filename after the binary-name prefix matches
/// a known target triple or the universal-binary pattern.
pub fn matches_target(after_prefix: &str) -> bool {
    after_prefix.starts_with("universal-apple-darwin")
        || TARGET_MAPPINGS
            .iter()
            .any(|(target, _, _)| after_prefix.starts_with(target))
}

/// Collect the binary targets from a Cargo manifest, optionally narrowing to a
/// single named binary. Errors if the manifest has no package, declares no
/// binaries, or names a `target_bin` that doesn't exist.
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

/// Build a [`FormulaContext`] for one binary from a Cargo manifest, with assets
/// left empty for a later fetch step to populate.
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
        executables: vec![format!("\"{}\"", binary.name)],
        homepage,
        version: package.version().to_string(),
        license,
        caveats: None,
        assets: Vec::new(), // we'll populate this later
    })
}

/// Build a [`FormulaContext`] from a `formulaic.toml` manifest, expanding the
/// optional `[bins]` table into install entries. Assets are left empty for a
/// later fetch step to populate.
pub fn create_base_context_from_generic(manifest: &GenericManifest) -> FormulaContext {
    use heck::ToUpperCamelCase;

    let executables = match &manifest.bins {
        Some(bins) => bins
            .iter()
            .map(|(name, path)| {
                if name == path {
                    format!("\"{name}\"")
                } else {
                    format!("\"{path}\" => \"{name}\"")
                }
            })
            .collect(),
        None => vec![format!("\"{}\"", manifest.name)],
    };

    FormulaContext {
        package: manifest.name.to_upper_camel_case(),
        description: manifest.description.clone().unwrap_or_default(),
        executable: manifest.name.clone(),
        executables,
        homepage: manifest.homepage.clone().unwrap_or_default(),
        version: manifest.version.clone(),
        license: manifest.license.clone().unwrap_or_else(|| "unlicensed".to_string()),
        caveats: manifest.caveats.clone(),
        assets: Vec::new(),
    }
}

/// Validated fields extracted from a GitHub release asset.
struct RawAssetInfo {
    filename: String,
    url: String,
    digest: String,
}

/// Extract and validate filename, URL, and digest from a release asset.
fn extract_asset_info(v: &roctogen::models::ReleaseAsset) -> anyhow::Result<RawAssetInfo> {
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

    let digest = v
        .digest
        .as_deref()
        .and_then(|d| d.split_once(':').map(|(_, hash)| hash.to_owned()));

    let digest = match digest {
        Some(d) => d,
        None => find_digest(filename, url).with_context(|| format!("Cannot calculate digest for asset {filename}"))?,
    };

    Ok(RawAssetInfo {
        filename: filename.clone(),
        url: url.clone(),
        digest,
    })
}

impl Asset {
    /// Create assets from a release asset that may be a universal binary.
    /// Universal binaries produce two assets (arm + intel); others produce one.
    pub fn assets_from_release_asset(v: &roctogen::models::ReleaseAsset) -> anyhow::Result<Vec<Self>> {
        let info = extract_asset_info(v)?;

        let platforms = extract_platforms(&info.filename);
        if platforms.is_empty() {
            anyhow::bail!("Cannot determine platform for asset {}", info.filename);
        }

        Ok(platforms
            .into_iter()
            .map(|(os, cpu)| Self {
                cpu: cpu.to_string(),
                os: os.to_string(),
                digest: info.digest.clone(),
                url: info.url.clone(),
            })
            .collect())
    }
}

fn asset_to_value(cpu: &str, os: &str, digest: &str, url: &str) -> upon::Value {
    [
        ("cpu".to_string(), cpu.to_string()),
        ("os".to_string(), os.to_string()),
        ("sha256".to_string(), digest.to_string()),
        ("url".to_string(), url.to_string()),
    ]
    .into_iter()
    .collect::<BTreeMap<String, String>>()
    .into()
}

impl From<Asset> for upon::Value {
    fn from(v: Asset) -> Self {
        asset_to_value(&v.cpu, &v.os, &v.digest, &v.url)
    }
}

impl From<&Asset> for upon::Value {
    fn from(v: &Asset) -> Self {
        asset_to_value(&v.cpu, &v.os, &v.digest, &v.url)
    }
}

/// Pull a 64-character hex SHA-256 digest out of the contents of a `.sha256`
/// sidecar file. Handles a bare hash, a `sha256:`-prefixed hash, the
/// `sha256sum`/`shasum` layout (`<hash>  filename`), and the BSD layout
/// (`SHA256 (filename) = <hash>`), with or without surrounding whitespace.
fn parse_sha256(contents: &str) -> Option<String> {
    contents
        .split(|c: char| c.is_whitespace() || c == '=' || c == ':')
        .find(|token| token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|token| token.to_ascii_lowercase())
}

/// Resolve the SHA-256 digest for an asset, trying in order: a local
/// `<filename>.sha256` sidecar, a local copy of the tarball, and finally
/// downloading the asset and hashing it.
pub fn find_digest(filename: &str, url: &str) -> anyhow::Result<String> {
    // Prefer a local shasum sidecar file if one is present and parseable.
    let digestpath = format!("{filename}.sha256");
    if let Ok(true) = std::fs::exists(&digestpath)
        && let Ok(contents) = std::fs::read_to_string(&digestpath)
        && let Some(hash) = parse_sha256(&contents)
    {
        return Ok(hash);
    }

    // try a local tarball
    if let Ok(mut fp) = std::fs::File::open(filename) {
        let mut buffer: Vec<u8> = Vec::new();
        if fp.read_to_end(&mut buffer).is_ok() {
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

/// Render a Homebrew formula from `context`.
///
/// `custom_template` overrides the built-in template entirely. Otherwise the
/// built-in template is used, and when `use_gh` is set the gh-cli download
/// strategy is woven in: the strategy class is prepended and every `url` line
/// opts into it via `using:`. The gh-cli strategy is never layered onto a
/// custom template — a custom template owns its own download handling.
pub fn render_to_string(
    use_gh: bool,
    custom_template: Option<&str>,
    context: &FormulaContext,
) -> anyhow::Result<String> {
    let inject_gh = use_gh && custom_template.is_none();

    let mut engine = upon::Engine::new();
    engine.add_template("formula", custom_template.unwrap_or(FORMULA_TMPL))?;

    // Convert FormulaContext to upon::Value
    let mut map: BTreeMap<&str, upon::Value> = BTreeMap::new();
    map.insert("package", context.package.as_str().into());
    map.insert("description", context.description.as_str().into());
    map.insert("executable", context.executable.as_str().into());
    map.insert("homepage", context.homepage.as_str().into());
    map.insert("version", context.version.as_str().into());
    map.insert("license", context.license.as_str().into());
    map.insert("executables", context.executables.iter().map(|s| s.as_str()).collect());
    map.insert("caveats", context.caveats.as_deref().unwrap_or_default().into());
    map.insert("assets", context.assets.iter().map(upon::Value::from).collect());
    // Appended to each `url` line; empty unless the gh-cli strategy is active.
    map.insert(
        "using_strategy",
        if inject_gh {
            ", using: GitHubCliDownloadStrategy"
        } else {
            ""
        }
        .into(),
    );

    let values = upon::to_value(map)?;
    let body = engine.template("formula").render(&values).to_string()?;

    if inject_gh {
        Ok(format!("{GH_DOWNLOAD_STRATEGY}\n{body}"))
    } else {
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_sha256;

    const HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn parses_bare_hash() {
        assert_eq!(parse_sha256(HASH).as_deref(), Some(HASH));
    }

    #[test]
    fn parses_hash_with_trailing_newline() {
        // The old length-based parser silently failed on this common shape.
        assert_eq!(parse_sha256(&format!("{HASH}\n")).as_deref(), Some(HASH));
    }

    #[test]
    fn parses_shasum_layout() {
        assert_eq!(
            parse_sha256(&format!("{HASH}  formulaic.tar.gz\n")).as_deref(),
            Some(HASH)
        );
    }

    #[test]
    fn parses_sha256_prefix() {
        assert_eq!(parse_sha256(&format!("sha256:{HASH}")).as_deref(), Some(HASH));
    }

    #[test]
    fn parses_bsd_layout() {
        assert_eq!(
            parse_sha256(&format!("SHA256 (formulaic.tar.gz) = {HASH}\n")).as_deref(),
            Some(HASH)
        );
    }

    #[test]
    fn uppercase_is_normalized_to_lowercase() {
        assert_eq!(parse_sha256(&HASH.to_uppercase()).as_deref(), Some(HASH));
    }

    #[test]
    fn rejects_non_digest_content() {
        assert_eq!(parse_sha256("no digest here\n"), None);
    }
}
