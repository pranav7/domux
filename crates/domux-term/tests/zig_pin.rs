#![cfg(feature = "ghostty")]
use std::fs;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

/// The Ghostty tree the build script resolved, so this checks the source that was built
/// rather than a second copy that could drift from it.
fn ghostty_src() -> &'static Path {
    Path::new(env!("DOMUX_GHOSTTY_SRC"))
}

#[test]
fn zig_pin_matches_ghostty_minimum_zig_version() {
    let zon = fs::read_to_string(ghostty_src().join("build.zig.zon")).unwrap();
    let line = zon
        .lines()
        .find(|l| l.contains("minimum_zig_version"))
        .expect("minimum_zig_version line");
    let zon_version = line.split('"').nth(1).expect("quoted version");
    let pin: toml::Value =
        toml::from_str(&fs::read_to_string(root().join("vendor/zig-pin.toml")).unwrap()).unwrap();
    assert_eq!(pin["version"].as_str().unwrap(), zon_version);
    for host in [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
    ] {
        let h = &pin["hosts"][host];
        assert!(
            h["url"].as_str().unwrap().contains(zon_version),
            "{host} url"
        );
        assert_eq!(h["sha256"].as_str().unwrap().len(), 64, "{host} sha256");
    }
}
