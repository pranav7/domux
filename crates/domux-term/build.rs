//! Builds the vendored libghostty-vt with a pinned Zig when the `ghostty` feature is on.
//!
//! Zig runs on the host and cross-compiles for the cargo target, so cross builds work as
//! long as the Rust linker for the target is available. Zig is fetched once into
//! `$XDG_CACHE_HOME/domux/zig/<version>` (default `~/.cache/domux/zig/<version>`), verified
//! against the sha256 in vendor/zig-pin.toml, and extracted with an atomic rename.
//! `DOMUX_ZIG=/path/to/zig` skips the download and must be the pinned version.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The zig build step that installs libghostty-vt. Ghostty installs the static library from
/// its default install step when `-Demit-lib-vt=true` is set; there is no separate step
/// (confirm with `zig build -l` in vendor/ghostty after a vendor bump).
const ZIG_VT_STEP: &str = "install";

/// Puts Ghostty's build into libghostty-vt-only mode: no macOS app, no docs.
const ZIG_VT_OPTION: &str = "-Demit-lib-vt=true";

/// Ghostty turns the xcframework on whenever `xcodebuild` is on PATH, and the Command Line
/// Tools stub fails without full Xcode. We only need the static library, so turn it off.
const ZIG_NO_XCFRAMEWORK: &str = "-Demit-xcframework=false";

fn main() {
    if env::var_os("CARGO_FEATURE_GHOSTTY").is_none() {
        return;
    }
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../..")
        .canonicalize()
        .unwrap();
    let vendor = root.join("vendor/ghostty");
    let pin_path = root.join("vendor/zig-pin.toml");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    for p in ["build.zig", "build.zig.zon", "src", "include", "pkg"] {
        println!("cargo:rerun-if-changed={}", vendor.join(p).display());
    }
    println!("cargo:rerun-if-changed={}", pin_path.display());
    println!("cargo:rerun-if-env-changed=DOMUX_ZIG");

    let pin: toml::Value = toml::from_str(&fs::read_to_string(&pin_path).unwrap()).unwrap();
    let version = pin["version"].as_str().unwrap().to_string();
    let zig = zig_binary(&pin, &version);

    let target = env::var("TARGET").unwrap();
    let zig_target = match target.as_str() {
        "aarch64-apple-darwin" => "aarch64-macos",
        "x86_64-apple-darwin" => "x86_64-macos",
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        other => panic!(
            "no zig target mapping for {other}; V2 supports macOS and Linux on arm64 and x86_64"
        ),
    };
    let prefix = out_dir.join("ghostty");
    let global_cache = env::var_os("ZIG_GLOBAL_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_root().join("global"));
    // Fetch Ghostty's Zig dependencies one at a time first. Zig fetches them in parallel
    // during a normal build, and parallel TLS handshakes fail on some machines with
    // "TlsInitializationFailed"; a serialized pass fills the global cache reliably, and the
    // build below then finds everything cached.
    let fetch = Command::new(&zig)
        .current_dir(&vendor)
        .arg("build")
        .arg("--fetch")
        .arg("-j1")
        .arg("--cache-dir")
        .arg(out_dir.join("zig-cache"))
        .arg("--global-cache-dir")
        .arg(&global_cache)
        .status()
        .expect("failed to start zig");
    assert!(
        fetch.success(),
        "zig build --fetch failed (zig {version} at {})",
        zig.display()
    );

    let status = Command::new(&zig)
        .current_dir(&vendor)
        .arg("build")
        .arg(ZIG_VT_STEP)
        .arg(ZIG_VT_OPTION)
        .arg(ZIG_NO_XCFRAMEWORK)
        .arg("-Doptimize=ReleaseFast")
        .arg(format!("-Dtarget={zig_target}"))
        .arg("--prefix")
        .arg(&prefix)
        .arg("--cache-dir")
        .arg(out_dir.join("zig-cache"))
        .arg("--global-cache-dir")
        .arg(&global_cache)
        .status()
        .expect("failed to start zig");
    assert!(
        status.success(),
        "zig build {ZIG_VT_STEP} failed (zig {version} at {})",
        zig.display()
    );

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    match env::var("CARGO_CFG_TARGET_OS").unwrap().as_str() {
        "macos" => println!("cargo:rustc-link-lib=c++"),
        "linux" => println!("cargo:rustc-link-lib=stdc++"),
        _ => {}
    }
}

fn cache_root() -> PathBuf {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var("HOME").unwrap()).join(".cache"));
    base.join("domux/zig")
}

fn zig_version_of(bin: &Path) -> String {
    let out = Command::new(bin)
        .arg("version")
        .output()
        .expect("run zig version");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn zig_binary(pin: &toml::Value, version: &str) -> PathBuf {
    if let Some(p) = env::var_os("DOMUX_ZIG") {
        let p = PathBuf::from(p);
        let have = zig_version_of(&p);
        assert_eq!(
            have, version,
            "DOMUX_ZIG points at zig {have}, but vendor/zig-pin.toml pins {version}"
        );
        return p;
    }
    let dir = cache_root().join(version);
    let bin = dir.join("zig");
    if bin.exists() {
        return bin;
    }
    let host = env::var("HOST").unwrap();
    let entry = &pin["hosts"][host.as_str()];
    let url = entry["url"]
        .as_str()
        .unwrap_or_else(|| panic!("no zig download for host {host} in vendor/zig-pin.toml"));
    let want = entry["sha256"].as_str().unwrap();

    fs::create_dir_all(cache_root()).unwrap();
    let tarball = cache_root().join(format!("zig-{version}-{host}.tar.xz"));
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&tarball)
        .arg(url)
        .status()
        .expect("run curl");
    assert!(status.success(), "downloading {url} failed");
    let got = hex(&Sha256::digest(fs::read(&tarball).unwrap()));
    assert_eq!(got, want, "sha256 mismatch for {url}");

    let tmp = cache_root().join(format!("{version}.tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let status = Command::new("tar")
        .arg("-xJf")
        .arg(&tarball)
        .arg("-C")
        .arg(&tmp)
        .arg("--strip-components=1")
        .status()
        .expect("run tar");
    assert!(status.success(), "extracting {} failed", tarball.display());
    match fs::rename(&tmp, &dir) {
        Ok(()) => {}
        Err(_) if dir.exists() => {
            let _ = fs::remove_dir_all(&tmp); // another build won the race
        }
        Err(e) => panic!("moving zig into place: {e}"),
    }
    let _ = fs::remove_file(&tarball);
    bin
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
