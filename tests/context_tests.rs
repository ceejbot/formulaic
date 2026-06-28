use std::path::PathBuf;

use cargo_toml::Manifest;
use formulaic::{
    Asset, AssetMatcher, BinaryInfo, FormulaContext, GenericManifest, create_base_context,
    create_base_context_from_generic, get_binaries_from_manifest, parse_owner_repo, render_to_string,
};

fn load_fixture(name: &str) -> Manifest {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(format!("{name}.toml"));
    Manifest::from_path(path).unwrap_or_else(|_| panic!("Failed to load fixture {name}"))
}

fn load_generic_fixture(name: &str) -> GenericManifest {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(format!("{name}.toml"));
    GenericManifest::from_path(&path).unwrap_or_else(|_| panic!("Failed to load generic fixture {name}"))
}

#[test]
fn can_render_template() {
    let context = FormulaContext {
        package: "Frobber".to_string(),
        description: "Frobs the whizzbanger".to_string(),
        version: "1.0.5".to_string(),
        license: "MIT".to_string(),
        homepage: "https://example.com".to_string(),
        executable: "frobber".to_string(),
        executables: vec!["\"frobber\"".to_string()],
        caveats: None,
        assets: vec![
            Asset {
                os: "mac".to_string(),
                cpu: "arm".to_string(),
                url: "https://example.com/".to_string(),
                digest: "deadbeef".to_string(),
            },
            Asset {
                os: "linux".to_string(),
                cpu: "intel".to_string(),
                url: "https://example.com/".to_string(),
                digest: "cafed00d".to_string(),
            },
        ],
    };
    let rendered = render_to_string(false, None, &context).expect("rendering the template failed");
    eprintln!("{rendered}");
    assert!(rendered.contains("sha256 \"cafed00d\""));
    assert!(rendered.contains("if OS.mac? && Hardware::CPU.arm?"));
    assert!(rendered.contains("bin.install \"frobber\""));
}

#[test]
fn can_render_gh_strategy() {
    let context = FormulaContext {
        package: "Frobber".to_string(),
        description: "Frobs the whizzbanger".to_string(),
        version: "1.0.5".to_string(),
        license: "MIT".to_string(),
        homepage: "https://example.com".to_string(),
        executable: "frobber".to_string(),
        executables: vec!["\"frobber\"".to_string()],
        caveats: None,
        assets: vec![Asset {
            os: "mac".to_string(),
            cpu: "arm".to_string(),
            url: "https://example.com/frobber-aarch64-apple-darwin.tar.gz".to_string(),
            digest: "deadbeef".to_string(),
        }],
    };
    let rendered = render_to_string(true, None, &context).expect("gh strategy rendering failed");

    // The download-strategy class is prepended...
    assert!(rendered.contains("class GitHubCliDownloadStrategy < CurlDownloadStrategy"));
    // ...and each url line opts into it.
    assert!(rendered.contains("using: GitHubCliDownloadStrategy"));
    // The portable gh lookup replaced the hardcoded /opt/homebrew path.
    assert!(rendered.contains("which(\"gh\")"));
    assert!(!rendered.contains("/opt/homebrew/bin/gh"));
    // The formula body still renders.
    assert!(rendered.contains("class Frobber < Formula"));
    assert!(rendered.contains("bin.install \"frobber\""));
}

#[test]
fn gh_strategy_is_not_layered_onto_a_custom_template() {
    let context = FormulaContext {
        package: "MyTool".to_string(),
        description: "A test tool".to_string(),
        version: "2.0.0".to_string(),
        license: "MIT".to_string(),
        homepage: "https://example.com".to_string(),
        executable: "my-tool".to_string(),
        executables: vec!["\"my-tool\"".to_string()],
        caveats: None,
        assets: vec![Asset {
            os: "mac".to_string(),
            cpu: "arm".to_string(),
            url: "https://example.com/asset.tar.gz".to_string(),
            digest: "abc123".to_string(),
        }],
    };
    let custom = "class {{ package }} < Formula\n    version \"{{ version }}\"\nend\n";

    // Even with use_gh = true, a custom template owns its own download handling.
    let rendered = render_to_string(true, Some(custom), &context).expect("custom render failed");
    assert!(!rendered.contains("GitHubCliDownloadStrategy"));
    assert!(rendered.contains("class MyTool < Formula"));
}

#[test]
fn can_render_custom_template() {
    let context = FormulaContext {
        package: "MyTool".to_string(),
        description: "A test tool".to_string(),
        version: "2.0.0".to_string(),
        license: "MIT".to_string(),
        homepage: "https://example.com".to_string(),
        executable: "my-tool".to_string(),
        executables: vec!["\"my-tool\"".to_string()],
        caveats: None,
        assets: vec![Asset {
            os: "mac".to_string(),
            cpu: "arm".to_string(),
            url: "https://example.com/asset.tar.gz".to_string(),
            digest: "abc123".to_string(),
        }],
    };

    let custom = r#"# Custom template for {{ executable }}
class {{ package }} < Formula
    version "{{ version }}"
end
"#;

    let rendered = render_to_string(false, Some(custom), &context).expect("custom template rendering failed");
    assert!(rendered.contains("# Custom template for my-tool"));
    assert!(rendered.contains("class MyTool < Formula"));
    assert!(rendered.contains("version \"2.0.0\""));
}

#[test]
fn single_binary_detection() {
    let manifest = load_fixture("single-binary");
    let binaries = get_binaries_from_manifest(&manifest, None).expect("text fixtures should behave");

    assert_eq!(binaries.len(), 1);
    assert_eq!(binaries[0].name, "example-tool");
    assert_eq!(binaries[0].package_name, "example-tool");
}

#[test]
fn multi_binary_detection() {
    let manifest = load_fixture("multi-binary");
    let binaries = get_binaries_from_manifest(&manifest, None).expect("text fixtures should behave");

    assert_eq!(binaries.len(), 3);

    let names: Vec<&str> = binaries.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"multi-tool"));
    assert!(names.contains(&"helper"));
    assert!(names.contains(&"converter"));
}

#[test]
fn specific_binary_filtering() {
    let manifest = load_fixture("multi-binary");
    let binaries = get_binaries_from_manifest(&manifest, Some("helper")).expect("text fixtures should behave");

    assert_eq!(binaries.len(), 1);
    assert_eq!(binaries[0].name, "helper");
}

#[test]
fn binary_not_found() {
    let manifest = load_fixture("multi-binary");
    let result = get_binaries_from_manifest(&manifest, Some("nonexistent"));

    assert!(result.is_err());
    let the_error = result.expect_err("we expected this check to fail");
    assert!(the_error.to_string().contains("Binary 'nonexistent' not found"));
}

#[test]
fn can_create_base_context() {
    let manifest = load_fixture("single-binary");
    let binary = BinaryInfo {
        name: "example-tool".to_string(),
        package_name: "example-tool".to_string(),
    };

    let context = create_base_context(&manifest, &binary).expect("text fixtures should behave");

    assert_eq!(context.package, "ExampleTool"); // UpperCamelCase
    assert_eq!(context.description, "A simple example tool");
    assert_eq!(context.executable, "example-tool");
    assert_eq!(context.homepage, "https://example.com");
    assert_eq!(context.version, "1.2.3");
    assert_eq!(context.license, "MIT");
    assert!(context.assets.is_empty()); // Should be empty initially
}

#[test]
fn asset_matcher_homebrew_platforms() {
    let matcher = AssetMatcher::new();

    // Test macOS platforms
    assert_eq!(
        matcher.extract_platform("example-aarch64-apple-darwin.tar.gz"),
        Some(("mac", "arm"))
    );
    assert_eq!(
        matcher.extract_platform("example-x86_64-apple-darwin.tar.gz"),
        Some(("mac", "intel"))
    );

    // Test Linux platforms
    assert_eq!(
        matcher.extract_platform("example-x86_64-unknown-linux-gnu.tar.gz"),
        Some(("linux", "intel"))
    );
    assert_eq!(
        matcher.extract_platform("example-aarch64-unknown-linux-gnu.tar.gz"),
        Some(("linux", "arm"))
    );

    // Test unsupported platforms
    assert_eq!(matcher.extract_platform("example-x86_64-pc-windows-msvc.zip"), None);
    assert_eq!(matcher.extract_platform("example-unknown-target.tar.gz"), None);
}

#[test]
fn asset_matcher_with_different_naming() {
    let matcher = AssetMatcher::new();

    // Test with different binary names
    assert_eq!(
        matcher.extract_platform("my-cli-aarch64-apple-darwin.tar.gz"),
        Some(("mac", "arm"))
    );
    assert_eq!(
        matcher.extract_platform("super_tool_x86_64-unknown-linux-gnu.tar.gz"),
        Some(("linux", "intel"))
    );
}

#[test]
fn asset_matcher_universal_binary() {
    let matcher = AssetMatcher::new();

    // Universal binary should be detected
    assert!(matcher.is_universal("paletter-universal-apple-darwin.tar.gz"));
    assert!(!matcher.is_universal("paletter-aarch64-apple-darwin.tar.gz"));

    // extract_platforms should return both arm and intel for universal
    let platforms = matcher.extract_platforms("paletter-universal-apple-darwin.tar.gz");
    assert_eq!(platforms.len(), 2);
    assert!(platforms.contains(&("mac", "arm")));
    assert!(platforms.contains(&("mac", "intel")));

    // Non-universal should return single platform
    let platforms = matcher.extract_platforms("paletter-aarch64-apple-darwin.tar.gz");
    assert_eq!(platforms.len(), 1);
    assert_eq!(platforms[0], ("mac", "arm"));
}

#[test]
fn asset_matcher_matches_target() {
    let matcher = AssetMatcher::new();

    assert!(matcher.matches_target("aarch64-apple-darwin.tar.gz"));
    assert!(matcher.matches_target("x86_64-unknown-linux-gnu.tar.gz"));
    assert!(matcher.matches_target("universal-apple-darwin.tar.gz"));
    assert!(!matcher.matches_target("windows-msvc.zip"));
    assert!(!matcher.matches_target("something-else.tar.gz"));
}

#[test]
fn context_with_minimal_package_info() {
    let manifest = load_fixture("single-binary");
    let binary = BinaryInfo {
        name: "example-tool".to_string(),
        package_name: "example-tool".to_string(),
    };

    let context = create_base_context(&manifest, &binary).expect("text fixtures should behave");

    // Verify all required fields are populated
    assert!(!context.package.is_empty());
    assert!(!context.executable.is_empty());
    assert!(!context.version.is_empty());
    // Description and homepage might be empty, that's ok
    // License should default to "unlicensed" if missing
}

// --- GenericManifest tests ---

#[test]
fn generic_manifest_full() {
    let manifest = load_generic_fixture("generic-manifest");

    assert_eq!(manifest.name, "paletter");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(
        manifest.description.as_deref(),
        Some("Convert tinty YAML color palettes to macOS .clr files")
    );
    assert_eq!(
        manifest.homepage.as_deref(),
        Some("https://github.com/ceejbot/paletter")
    );
    assert_eq!(manifest.license.as_deref(), Some("Parity-7.0.0"));
    assert_eq!(
        manifest.repository.as_deref(),
        Some("https://github.com/ceejbot/paletter")
    );
}

#[test]
fn generic_manifest_minimal() {
    let manifest = load_generic_fixture("generic-minimal");

    assert_eq!(manifest.name, "minimal-tool");
    assert_eq!(manifest.version, "0.1.0");
    assert!(manifest.description.is_none());
    assert!(manifest.homepage.is_none());
    assert!(manifest.license.is_none());
    assert!(manifest.repository.is_none());
}

#[test]
fn generic_manifest_owner_repo() {
    let manifest = load_generic_fixture("generic-manifest");
    let (owner, repo) = manifest.owner_repo().expect("should parse owner/repo");

    assert_eq!(owner, "ceejbot");
    assert_eq!(repo, "paletter");
}

#[test]
fn generic_manifest_owner_repo_missing() {
    let manifest = load_generic_fixture("generic-minimal");
    let result = manifest.owner_repo();
    assert!(result.is_err());
}

#[test]
fn create_context_from_generic_manifest() {
    let manifest = load_generic_fixture("generic-manifest");
    let context = create_base_context_from_generic(&manifest);

    assert_eq!(context.package, "Paletter");
    assert_eq!(context.executable, "paletter");
    assert_eq!(context.executables, vec!["\"paletter\""]);
    assert_eq!(
        context.description,
        "Convert tinty YAML color palettes to macOS .clr files"
    );
    assert_eq!(context.homepage, "https://github.com/ceejbot/paletter");
    assert_eq!(context.version, "1.0.0");
    assert_eq!(context.license, "Parity-7.0.0");
    assert!(context.assets.is_empty());
}

#[test]
fn create_context_from_generic_minimal() {
    let manifest = load_generic_fixture("generic-minimal");
    let context = create_base_context_from_generic(&manifest);

    assert_eq!(context.package, "MinimalTool");
    assert_eq!(context.executable, "minimal-tool");
    assert_eq!(context.executables, vec!["\"minimal-tool\""]);
    assert_eq!(context.description, "");
    assert_eq!(context.homepage, "");
    assert_eq!(context.version, "0.1.0");
    assert_eq!(context.license, "unlicensed");
}

#[test]
fn parse_owner_repo_https() {
    let (owner, repo) = parse_owner_repo("https://github.com/ceejbot/formulaic").expect("should parse");
    assert_eq!(owner, "ceejbot");
    assert_eq!(repo, "formulaic");
}

#[test]
fn parse_owner_repo_git_suffix() {
    let (owner, repo) = parse_owner_repo("https://github.com/ceejbot/formulaic.git").expect("should parse");
    assert_eq!(owner, "ceejbot");
    assert_eq!(repo, "formulaic");
}

#[test]
fn generic_manifest_with_gh_strategy() {
    let toml = r#"
        name = "test-tool"
        version = "1.0.0"
        gh-cli-strategy = true
    "#;
    let manifest: GenericManifest = toml::from_str(toml).expect("should parse");
    assert_eq!(manifest.use_gh_strategy, Some(true));
}

#[test]
fn generic_manifest_without_gh_strategy() {
    let toml = r#"
        name = "test-tool"
        version = "1.0.0"
    "#;
    let manifest: GenericManifest = toml::from_str(toml).expect("should parse");
    assert_eq!(manifest.use_gh_strategy, None);
}

#[test]
fn generic_manifest_gh_strategy_false() {
    let toml = r#"
        name = "test-tool"
        version = "1.0.0"
        gh-cli-strategy = false
    "#;
    let manifest: GenericManifest = toml::from_str(toml).expect("should parse");
    assert_eq!(manifest.use_gh_strategy, Some(false));
}

#[test]
#[ignore] // Workspace members need to be tested differently
fn workspace_member_context() {
    // This test is disabled because cargo_toml requires workspace members
    // to be part of an actual workspace directory structure
    // TODO: Create proper workspace test setup
}

#[test]
fn generic_manifest_with_bins() {
    let manifest = load_generic_fixture("generic-multibins");

    assert_eq!(manifest.name, "my-project");
    let bins = manifest.bins.as_ref().expect("bins should be present");
    assert_eq!(bins.len(), 2);
    assert_eq!(bins.get("tool-a").map(|s| s.as_str()), Some("tool-a"));
    assert_eq!(bins.get("tool-b").map(|s| s.as_str()), Some("bin/tool-b"));
}

#[test]
fn create_context_from_generic_with_bins() {
    let manifest = load_generic_fixture("generic-multibins");
    let context = create_base_context_from_generic(&manifest);

    assert_eq!(context.executable, "my-project");
    assert_eq!(context.executables.len(), 2);
    assert!(context.executables.contains(&"\"tool-a\"".to_string()));
    assert!(
        context
            .executables
            .contains(&"\"bin/tool-b\" => \"tool-b\"".to_string())
    );
}

#[test]
fn render_template_multiple_bins() {
    let context = FormulaContext {
        package: "MyProject".to_string(),
        description: "A multi-bin project".to_string(),
        version: "2.0.0".to_string(),
        license: "MIT".to_string(),
        homepage: "https://example.com".to_string(),
        executable: "my-project".to_string(),
        executables: vec!["\"tool-a\"".to_string(), "\"bin/tool-b\" => \"tool-b\"".to_string()],
        caveats: None,
        assets: vec![Asset {
            os: "mac".to_string(),
            cpu: "arm".to_string(),
            url: "https://example.com/asset.tar.gz".to_string(),
            digest: "abc123".to_string(),
        }],
    };

    let rendered = render_to_string(false, None, &context).expect("rendering failed");
    eprintln!("{rendered}");
    assert!(rendered.contains("bin.install \"tool-a\""));
    assert!(rendered.contains("bin.install \"bin/tool-b\" => \"tool-b\""));
}

#[test]
fn generic_bins_fallback() {
    let manifest = load_generic_fixture("generic-manifest");
    assert!(manifest.bins.is_none());

    let context = create_base_context_from_generic(&manifest);
    assert_eq!(context.executables, vec!["\"paletter\""]);
}

#[test]
fn generic_manifest_with_caveats() {
    let manifest = load_generic_fixture("generic-caveats");

    assert_eq!(manifest.name, "caveated-tool");
    assert_eq!(
        manifest.caveats.as_deref(),
        Some("You must add ~/.caveated-tool/bin to your PATH.")
    );
}

#[test]
fn generic_manifest_without_caveats() {
    let manifest = load_generic_fixture("generic-manifest");
    assert!(manifest.caveats.is_none());

    let context = create_base_context_from_generic(&manifest);
    assert!(context.caveats.is_none());
}

#[test]
fn render_template_with_caveats() {
    let context = FormulaContext {
        package: "CaveatedTool".to_string(),
        description: "A tool with caveats".to_string(),
        version: "1.0.0".to_string(),
        license: "MIT".to_string(),
        homepage: "https://example.com".to_string(),
        executable: "caveated-tool".to_string(),
        executables: vec!["\"caveated-tool\"".to_string()],
        caveats: Some("You must add ~/.caveated-tool/bin to your PATH.".to_string()),
        assets: vec![Asset {
            os: "mac".to_string(),
            cpu: "arm".to_string(),
            url: "https://example.com/asset.tar.gz".to_string(),
            digest: "abc123".to_string(),
        }],
    };

    let rendered = render_to_string(false, None, &context).expect("rendering failed");
    eprintln!("{rendered}");
    assert!(rendered.contains("def caveats"));
    assert!(rendered.contains("You must add ~/.caveated-tool/bin to your PATH."));
}

#[test]
fn render_template_without_caveats() {
    let context = FormulaContext {
        package: "NoCaveats".to_string(),
        description: "No caveats here".to_string(),
        version: "1.0.0".to_string(),
        license: "MIT".to_string(),
        homepage: "https://example.com".to_string(),
        executable: "no-caveats".to_string(),
        executables: vec!["\"no-caveats\"".to_string()],
        caveats: None,
        assets: vec![Asset {
            os: "mac".to_string(),
            cpu: "arm".to_string(),
            url: "https://example.com/asset.tar.gz".to_string(),
            digest: "abc123".to_string(),
        }],
    };

    let rendered = render_to_string(false, None, &context).expect("rendering failed");
    assert!(!rendered.contains("def caveats"));
}
