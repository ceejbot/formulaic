//! This is a typical Rust clap derive-style cli app.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use cargo_toml::Manifest;
use clap::Parser;
use clap::builder::Styles;
use clap::builder::styling::AnsiColor;
use formulaic::{
    Asset, AssetMatcher, FormulaContext, create_base_context, find_digest, get_binaries_from_manifest, render_to_string,
};
use roctogen::endpoints::repos;
use roctokit::adapters::client;
use roctokit::auth::Auth;

#[derive(Debug, Clone, Parser)]
#[clap(author, version, styles = v3_styles())]
/// Generates Homebrew formula files for Rust binaries from their Cargo manifest.
///
/// Requires a valid github token in GITHUB_ACCESS_TOKEN or GITHUB_TOKEN.
struct Args {
    /// Path to the Cargo.toml file for the installable binary
    #[arg(default_value = "./Cargo.toml")]
    manifest: String,
    /// Use the `gh` cli download strategy; useful for private tap repos
    #[arg(long = "gh-cli-strategy", short = 'g')]
    use_gh_strategy: bool,
    /// If you have no repo-reading API permissions, we'll use only local data
    #[arg(long = "local", short = 'l')]
    local: bool,
    /// Generate formula for specific binary only
    #[arg(long = "bin")]
    bin: Option<String>,
    /// Generate formulas for all binaries; default if there is more than one
    #[arg(long = "all")]
    all: bool,
    /// Output directory for formula files
    #[arg(long = "output-dir", short = 'o')]
    output_dir: Option<PathBuf>,
    /// Preview formulas without writing files
    #[arg(long = "dry-run")]
    dry_run: bool,
}

fn v3_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Green.on_default())
}

fn make_context_from_github(
    context: &mut FormulaContext,
    manifest: &Manifest,
    github: &roctokit::adapters::ureq::Client,
) -> anyhow::Result<()> {
    let Some(ref package) = manifest.package else {
        anyhow::bail!("The Rust project must have at least one package in it.");
    };

    let repository = package
        .repository()
        .with_context(|| "Package must have a repository field for GitHub API access")?;

    let mut chunks: Vec<&str> = repository.split('/').collect();
    let repo = chunks
        .pop()
        .with_context(|| "Invalid repository URL format")?
        .trim_end_matches(".git");
    let owner = chunks.pop().with_context(|| "Invalid repository URL format")?;

    // Gather release information
    let repo_api = repos::new(github);
    let latest_release = repo_api
        .get_latest_release(owner, repo)
        .map_err(|e| anyhow::anyhow!("Unable to get latest release for {}/{}: {:?}", owner, repo, e))?;

    let matcher = AssetMatcher::new();

    if let Some(ref assets) = latest_release.assets {
        for asset in assets {
            // Only include assets that match the current binary exactly
            if let Some(ref asset_name) = asset.name {
                let expected_prefix = format!("{}-", context.executable);
                if asset_name.starts_with(&expected_prefix) {
                    let after_prefix = &asset_name[expected_prefix.len()..];

                    // Check if what comes after the prefix starts with a target triple
                    let is_direct_target = matcher
                        .target_mappings
                        .keys()
                        .any(|target| after_prefix.starts_with(target));

                    if is_direct_target && let Ok(mapped) = Asset::from_release_asset(asset, &matcher) {
                        context.assets.push(mapped);
                    }
                }
            }
        }
    }

    if context.assets.is_empty() {
        anyhow::bail!("No compatible assets found for Homebrew platforms (macOS/Linux)");
    }

    Ok(())
}

fn make_context_local_new(
    context: &mut FormulaContext,
    manifest: &Manifest,
    manifest_path: &str,
) -> anyhow::Result<()> {
    let Some(ref package) = manifest.package else {
        anyhow::bail!("The Rust project must have at least one package in it.");
    };

    let version = package.version().to_string();
    let repository = package
        .repository()
        .with_context(|| "Package must have a repository field for local mode")?;

    let mut chunks: Vec<&str> = repository.split('/').collect();
    let repo = chunks
        .pop()
        .with_context(|| "Invalid repository URL format")?
        .trim_end_matches(".git");
    let owner = chunks.pop().with_context(|| "Invalid repository URL format")?;

    // Look for .tar.gz files in dist/ directory at the same level as Cargo.toml
    let mut dir = PathBuf::from(manifest_path);
    dir.pop();
    dir.push("dist");

    if !dir.is_dir() {
        anyhow::bail!("No dist/ directory found next to Cargo.toml for local mode");
    }

    let matcher = AssetMatcher::new();

    for entry in std::fs::read_dir(&dir).with_context(|| format!("Failed to read dist directory: {}", dir.display()))? {
        let entry = entry?;
        let fullpath = entry.path();

        if fullpath.is_file() && fullpath.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("gz")) {
            let Some(basename) = fullpath.file_name() else {
                continue;
            };

            let basename_str = basename.to_string_lossy();

            // Check if this asset matches our target platforms AND current binary exactly
            let expected_prefix = format!("{}-", context.executable);
            if basename_str.starts_with(&expected_prefix) {
                // For "formulaic-", we want to match "formulaic-aarch64-apple-darwin.tar.gz"
                // but NOT "formulaic-helper-aarch64-apple-darwin.tar.gz"
                let after_prefix = &basename_str[expected_prefix.len()..];

                // Check if what comes after the prefix starts with a target triple
                let is_direct_target = matcher
                    .target_mappings
                    .keys()
                    .any(|target| after_prefix.starts_with(target));

                if is_direct_target && let Some((os, cpu)) = matcher.extract_platform(&basename_str) {
                    let url = format!("https://github.com/{owner}/{repo}/releases/download/v{version}/{basename_str}");

                    let path_str = fullpath.to_string_lossy();
                    if let Ok(digest) = find_digest(&path_str, &url) {
                        context.assets.push(Asset {
                            cpu: cpu.to_string(),
                            os: os.to_string(),
                            digest,
                            url,
                        });
                    }
                }
            }
        }
    }

    if context.assets.is_empty() {
        anyhow::bail!("No compatible assets found in dist/ directory for Homebrew platforms");
    }

    Ok(())
}

fn render_formula(
    use_gh: bool,
    context: &FormulaContext,
    output_dir: Option<&PathBuf>,
    dry_run: bool,
) -> anyhow::Result<String> {
    let rendered = render_to_string(use_gh, context)?;
    let formula_filename = format!("{}.rb", context.executable);

    let formula_path = if let Some(dir) = output_dir {
        dir.join(&formula_filename)
    } else {
        PathBuf::from(&formula_filename)
    };

    if dry_run {
        println!("Would write {}:", formula_path.display());
        println!("{rendered}");
        println!("{}", "=".repeat(80));
    } else {
        if let Some(parent) = formula_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create output directory {}", parent.display()))?;
        }

        let mut fp = std::fs::File::create(&formula_path)
            .with_context(|| format!("Failed to create formula file {}", formula_path.display()))?;
        let count = fp.write(rendered.as_bytes())?;
        if count == 0 {
            anyhow::bail!("zero-length formula file indicates trouble in River City.");
        }
    }

    Ok(formula_path.to_string_lossy().to_string())
}

/// Parse arguments and act.
fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Get GitHub token only if we're not in local mode
    let github_client = if !args.local {
        let token = std::env::var("GITHUB_ACCESS_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .with_context(|| "GitHub token required in GITHUB_ACCESS_TOKEN or GITHUB_TOKEN")?;

        let auth = Auth::Token(token);
        Some(client(&auth)?)
    } else {
        None
    };

    let manifest = cargo_toml::Manifest::from_path(&args.manifest)
        .with_context(|| format!("Failed to read manifest at {}", args.manifest))?;

    // Get all binaries we should process
    let binaries = get_binaries_from_manifest(&manifest, args.bin.as_deref())?;

    // Check if we should process all binaries or just one
    let process_all = args.all || (binaries.len() > 1 && args.bin.is_none());
    let binaries_to_process = if process_all {
        &binaries[..]
    } else {
        &binaries[..1] // Just the first one
    };

    let mut generated_files = Vec::new();

    for binary in binaries_to_process {
        let mut context = create_base_context(&manifest, binary)?;

        // Populate assets based on mode
        if args.local {
            make_context_local_new(&mut context, &manifest, &args.manifest)?;
        } else if let Some(ref github) = github_client {
            make_context_from_github(&mut context, &manifest, github)?;
        } else {
            anyhow::bail!("GitHub client not available");
        }

        let formula_path = render_formula(args.use_gh_strategy, &context, args.output_dir.as_ref(), args.dry_run)?;

        generated_files.push(formula_path);
    }

    if !args.dry_run {
        if generated_files.len() == 1 {
            println!("{}", generated_files[0]);
        } else {
            println!("Generated {} formula files:", generated_files.len());
            for file in &generated_files {
                println!("  {file}");
            }
        }
    }

    Ok(())
}
