//! Builds libghostty-vt with a pinned Zig when the `ghostty` feature is on.
//!
//! Neither input lives in this repository. Both are pinned, cached under
//! `$XDG_CACHE_HOME/domux` (default `~/.cache/domux`), and installed with an atomic rename:
//!
//! - Ghostty source: cloned at the commit in vendor/ghostty-pin.toml. A git object name is
//!   content addressed, so checking out the commit is the integrity check.
//!   `DOMUX_GHOSTTY_SOURCE_DIR=/path/to/ghostty` uses a local checkout instead, which is how
//!   you build offline or against local edits.
//! - Zig: downloaded and verified against the sha256 in vendor/zig-pin.toml.
//!   `DOMUX_ZIG=/path/to/zig` skips the download and must be the pinned version.
//!
//! Zig runs on the host and cross-compiles for the cargo target, so cross builds work as
//! long as the Rust linker for the target is available.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The zig build step that installs libghostty-vt. Ghostty installs the static library from
/// its default install step when `-Demit-lib-vt=true` is set; there is no separate step
/// (confirm with `zig build -l` in the resolved source tree after a pin bump).
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
    let pin_path = root.join("vendor/zig-pin.toml");
    let ghostty_pin_path = root.join("vendor/ghostty-pin.toml");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", pin_path.display());
    println!("cargo:rerun-if-changed={}", ghostty_pin_path.display());
    println!("cargo:rerun-if-env-changed=DOMUX_ZIG");
    println!("cargo:rerun-if-env-changed=DOMUX_GHOSTTY_SOURCE_DIR");

    let ghostty_src = ghostty_source(&ghostty_pin_path);
    // The header fingerprint test and the binding regeneration read the same tree this
    // builds, so none of them can drift onto a different Ghostty.
    println!(
        "cargo:rustc-env=DOMUX_GHOSTTY_SRC={}",
        ghostty_src.display()
    );
    if env::var_os("DOMUX_GHOSTTY_SOURCE_DIR").is_some() {
        // A local checkout is edited between builds; a cached clone never is.
        for p in ["build.zig", "build.zig.zon", "src", "include", "pkg"] {
            println!("cargo:rerun-if-changed={}", ghostty_src.join(p).display());
        }
    }

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
        .unwrap_or_else(|| zig_cache().join("global"));
    // Fetch Ghostty's Zig dependencies one at a time first. Zig fetches them in parallel
    // during a normal build, and parallel TLS handshakes fail on some machines with
    // "TlsInitializationFailed"; a serialized pass fills the global cache reliably, and the
    // build below then finds everything cached. Pass the same options as the build: many of
    // Ghostty's dependencies are lazy, so a different option set resolves a different set of
    // them and leaves the build to fetch the rest in parallel.
    let fetch = Command::new(&zig)
        .current_dir(&ghostty_src)
        .arg("build")
        .arg("--fetch")
        .arg("-j1")
        .arg(ZIG_VT_OPTION)
        .arg(ZIG_NO_XCFRAMEWORK)
        .arg(format!("-Dtarget={zig_target}"))
        .arg("--cache-dir")
        .arg(out_dir.join("zig-cache"))
        .arg("--global-cache-dir")
        .arg(&global_cache)
        .status()
        .expect("failed to start zig");
    if !fetch.success() {
        // Best effort only. This pass exists to serialize downloads, not to gate the build:
        // the packages may already be cached, and lazy nested packages resolve during the
        // build regardless. Let the build below be the step that decides.
        println!(
            "cargo:warning=zig build --fetch did not finish; continuing, the build will fetch \
             anything still missing"
        );
    }

    let status = Command::new(&zig)
        .current_dir(&ghostty_src)
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

/// `$XDG_CACHE_HOME/domux` (default `~/.cache/domux`), where both pinned inputs land.
fn cache_root() -> PathBuf {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var("HOME").unwrap()).join(".cache"));
    base.join("domux")
}

fn zig_cache() -> PathBuf {
    cache_root().join("zig")
}

/// The Ghostty tree to build. `DOMUX_GHOSTTY_SOURCE_DIR` names a local checkout, which is
/// how you build offline or against local edits. Otherwise the pinned commit is cloned once
/// into `~/.cache/domux/ghostty/<commit>` and reused by every later build.
fn ghostty_source(pin_path: &Path) -> PathBuf {
    if let Some(dir) = env::var_os("DOMUX_GHOSTTY_SOURCE_DIR") {
        let dir = PathBuf::from(dir);
        assert!(
            dir.join("build.zig").exists(),
            "DOMUX_GHOSTTY_SOURCE_DIR has no build.zig: {}",
            dir.display()
        );
        return dir;
    }
    let pin: toml::Value = toml::from_str(&fs::read_to_string(pin_path).unwrap()).unwrap();
    let repo = pin["repo"]
        .as_str()
        .expect("repo in vendor/ghostty-pin.toml");
    let commit = pin["commit"]
        .as_str()
        .expect("commit in vendor/ghostty-pin.toml");
    let dir = cache_root().join("ghostty").join(commit);
    if dir.join("build.zig").exists() {
        return dir;
    }
    clone_ghostty(repo, commit, &dir);
    dir
}

/// Clones `commit` from `repo` into `dir`. A git object name is content addressed, so
/// checking the commit out is the integrity check; there is no separate checksum to verify.
/// `--filter=blob:none` fetches only the blobs the checkout needs. The clone lands in a
/// process-unique directory and is renamed into place, so concurrent builds never share a
/// path or see a half-written tree.
fn clone_ghostty(repo: &str, commit: &str, dir: &Path) {
    let parent = dir.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    let tmp = parent.join(format!("{commit}.tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);

    let status = Command::new("git")
        .args([
            "clone",
            "--quiet",
            "--filter=blob:none",
            "--no-checkout",
            repo,
        ])
        .arg(&tmp)
        .status()
        .expect("run git clone");
    assert!(status.success(), "cloning {repo} failed");

    let status = Command::new("git")
        .args(["checkout", "--quiet", "--detach", commit])
        .current_dir(&tmp)
        .status()
        .expect("run git checkout");
    assert!(
        status.success(),
        "{repo} has no commit {commit}; check vendor/ghostty-pin.toml"
    );

    match fs::rename(&tmp, dir) {
        Ok(()) => {}
        Err(_) if dir.exists() => {
            let _ = fs::remove_dir_all(&tmp); // another build won the race
        }
        Err(e) => panic!("moving the ghostty clone into place: {e}"),
    }
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
    let dir = zig_cache().join(version);
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

    fs::create_dir_all(zig_cache()).unwrap();
    let tarball = zig_cache().join(format!("zig-{version}-{host}.tar.xz"));
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&tarball)
        .arg(url)
        .status()
        .expect("run curl");
    assert!(status.success(), "downloading {url} failed");
    let got = hex(&Sha256::digest(fs::read(&tarball).unwrap()));
    assert_eq!(got, want, "sha256 mismatch for {url}");

    let tmp = zig_cache().join(format!("{version}.tmp-{}", std::process::id()));
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
