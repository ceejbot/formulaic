use std::path::PathBuf;

use cargo_toml::Manifest;
use formulaic::{
    Asset, AssetMatcher, BinaryInfo, FormulaContext, create_base_context, get_binaries_from_manifest, render_to_string,
};

fn load_fixture(name: &str) -> Manifest {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(format!("{name}.toml"));
    Manifest::from_path(path).unwrap_or_else(|_| panic!("Failed to load fixture {name}"))
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
    let rendered = render_to_string(false, &context).expect("rendering the template failed");
    eprintln!("{rendered}");
    assert!(rendered.contains("sha256 \"cafed00d\""));
    assert!(rendered.contains("bin.install \"frobber\" if OS.mac?"));
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

#[test]
#[ignore] // Workspace members need to be tested differently
fn workspace_member_context() {
    // This test is disabled because cargo_toml requires workspace members
    // to be part of an actual workspace directory structure
    // TODO: Create proper workspace test setup
}
