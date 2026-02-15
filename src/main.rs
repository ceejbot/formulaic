//! This is a typical Rust clap derive-style cli app.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use cargo_toml::Manifest;
use clap::Parser;
use clap::builder::Styles;
use clap::builder::styling::AnsiColor;
use formulaic::{
    Asset, AssetMatcher, FormulaContext, GenericManifest, create_base_context,
    create_base_context_from_generic, find_digest, get_binaries_from_manifest, parse_owner_repo,
    render_to_string,
};
use roctogen::endpoints::repos;
use roctokit::adapters::client;
use roctokit::auth::Auth;

#[derive(Debug, Clone, Parser)]
#[clap(author, version, styles = v3_styles())]
/// Generates Homebrew formula files for Rust binaries from their Cargo manifest,
/// or for any project with a formulaic.toml file.
///
/// Requires a valid github token in GITHUB_ACCESS_TOKEN or GITHUB_TOKEN.
struct Args {
    /// Path to the manifest file (Cargo.toml or formulaic.toml).
    /// If omitted, looks for formulaic.toml then Cargo.toml in the current directory.
    manifest: Option<String>,
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
    /// Path to a custom formula template file (upon/Jinja2 syntax)
    #[arg(long = "template", short = 't')]
    template: Option<PathBuf>,
}

fn v3_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Green.on_default())
}

/// What kind of manifest we resolved.
enum ResolvedManifest {
    Cargo { path: String, manifest: Box<Manifest> },
    Generic { manifest: GenericManifest },
}

/// Resolve which manifest to use.
///
/// 1. If a path is given explicitly, detect type by filename.
/// 2. Otherwise try `formulaic.toml` then `Cargo.toml` in cwd.
fn resolve_manifest(explicit: Option<&str>) -> anyhow::Result<ResolvedManifest> {
    if let Some(path) = explicit {
        if path.ends_with("formulaic.toml") {
            let manifest = GenericManifest::from_path(path.as_ref())?;
            return Ok(ResolvedManifest::Generic { manifest });
        }
        let manifest = Manifest::from_path(path)
            .with_context(|| format!("Failed to read manifest at {path}"))?;
        return Ok(ResolvedManifest::Cargo {
            path: path.to_string(),
            manifest: Box::new(manifest),
        });
    }

    // Auto-detect: try formulaic.toml first, then Cargo.toml
    let formulaic_path = PathBuf::from("./formulaic.toml");
    if formulaic_path.exists() {
        let manifest = GenericManifest::from_path(&formulaic_path)?;
        return Ok(ResolvedManifest::Generic { manifest });
    }

    let cargo_path = "./Cargo.toml";
    let manifest = Manifest::from_path(cargo_path)
        .with_context(|| "No formulaic.toml or Cargo.toml found in current directory")?;
    Ok(ResolvedManifest::Cargo {
        path: cargo_path.to_string(),
        manifest: Box::new(manifest),
    })
}

fn fetch_github_assets(
    context: &mut FormulaContext,
    owner: &str,
    repo: &str,
    github: &roctokit::adapters::ureq::Client,
) -> anyhow::Result<()> {
    let repo_api = repos::new(github);
    let latest_release = repo_api
        .get_latest_release(owner, repo)
        .map_err(|e| anyhow::anyhow!("Unable to get latest release for {}/{}: {:?}", owner, repo, e))?;

    let matcher = AssetMatcher::new();

    if let Some(ref assets) = latest_release.assets {
        for asset in assets {
            if let Some(ref asset_name) = asset.name {
                let expected_prefix = format!("{}-", context.executable);
                if asset_name.starts_with(&expected_prefix) {
                    let after_prefix = &asset_name[expected_prefix.len()..];

                    if matcher.matches_target(after_prefix)
                        && let Ok(mapped) = Asset::assets_from_release_asset(asset, &matcher)
                    {
                        context.assets.extend(mapped);
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

fn fetch_local_assets(
    context: &mut FormulaContext,
    owner: &str,
    repo: &str,
    version: &str,
    manifest_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let dist_dir = manifest_dir.join("dist");

    if !dist_dir.is_dir() {
        anyhow::bail!(
            "No dist/ directory found at {} for local mode",
            dist_dir.display()
        );
    }

    let matcher = AssetMatcher::new();

    for entry in
        std::fs::read_dir(&dist_dir).with_context(|| format!("Failed to read dist directory: {}", dist_dir.display()))?
    {
        let entry = entry?;
        let fullpath = entry.path();

        if fullpath.is_file() && fullpath.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("gz")) {
            let Some(basename) = fullpath.file_name() else {
                continue;
            };

            let basename_str = basename.to_string_lossy();

            let expected_prefix = format!("{}-", context.executable);
            if basename_str.starts_with(&expected_prefix) {
                let after_prefix = &basename_str[expected_prefix.len()..];

                if matcher.matches_target(after_prefix) {
                    let platforms = matcher.extract_platforms(&basename_str);
                    if !platforms.is_empty() {
                        let url = format!(
                            "https://github.com/{owner}/{repo}/releases/download/v{version}/{basename_str}"
                        );
                        let path_str = fullpath.to_string_lossy();
                        if let Ok(digest) = find_digest(&path_str, &url) {
                            for (os, cpu) in platforms {
                                context.assets.push(Asset {
                                    cpu: cpu.to_string(),
                                    os: os.to_string(),
                                    digest: digest.clone(),
                                    url: url.clone(),
                                });
                            }
                        }
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
    custom_template: Option<&str>,
    context: &FormulaContext,
    output_dir: Option<&PathBuf>,
    dry_run: bool,
) -> anyhow::Result<String> {
    let rendered = render_to_string(use_gh, custom_template, context)?;
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

    // Load custom template if specified
    let custom_template_content = if let Some(ref path) = args.template {
        Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read template file {}", path.display()))?,
        )
    } else {
        None
    };
    let custom_template = custom_template_content.as_deref();

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

    let resolved = resolve_manifest(args.manifest.as_deref())?;

    match resolved {
        ResolvedManifest::Generic { manifest } => {
            let (owner, repo) = manifest.owner_repo()?;
            let mut context = create_base_context_from_generic(&manifest);

            if args.local {
                // For generic manifests, use cwd as the manifest directory
                let manifest_dir = if let Some(ref path) = args.manifest {
                    let p = PathBuf::from(path);
                    p.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
                } else {
                    PathBuf::from(".")
                };
                fetch_local_assets(&mut context, &owner, &repo, &manifest.version, &manifest_dir)?;
            } else if let Some(ref github) = github_client {
                fetch_github_assets(&mut context, &owner, &repo, github)?;
            } else {
                anyhow::bail!("GitHub client not available");
            }

            let formula_path = render_formula(
                args.use_gh_strategy,
                custom_template,
                &context,
                args.output_dir.as_ref(),
                args.dry_run,
            )?;

            if !args.dry_run {
                println!("{formula_path}");
            }
        }
        ResolvedManifest::Cargo { path, manifest } => {
            let binaries = get_binaries_from_manifest(&manifest, args.bin.as_deref())?;

            let process_all = args.all || (binaries.len() > 1 && args.bin.is_none());
            let binaries_to_process = if process_all {
                &binaries[..]
            } else {
                &binaries[..1]
            };

            let Some(ref package) = manifest.package else {
                anyhow::bail!("The Rust project must have at least one package in it.");
            };

            let repository = package
                .repository()
                .with_context(|| "Package must have a repository field")?;
            let (owner, repo) = parse_owner_repo(repository)?;

            let mut generated_files = Vec::new();

            for binary in binaries_to_process {
                let mut context = create_base_context(&manifest, binary)?;

                let version = context.version.clone();
                if args.local {
                    let manifest_dir = PathBuf::from(&path);
                    let manifest_dir = manifest_dir
                        .parent()
                        .map(|d| d.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."));
                    fetch_local_assets(&mut context, &owner, &repo, &version, &manifest_dir)?;
                } else if let Some(ref github) = github_client {
                    fetch_github_assets(&mut context, &owner, &repo, github)?;
                } else {
                    anyhow::bail!("GitHub client not available");
                }

                let formula_path = render_formula(
                    args.use_gh_strategy,
                    custom_template,
                    &context,
                    args.output_dir.as_ref(),
                    args.dry_run,
                )?;

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
        }
    }

    Ok(())
}
