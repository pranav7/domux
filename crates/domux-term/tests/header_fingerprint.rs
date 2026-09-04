#![cfg(feature = "ghostty")]
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Every `.h` under `vendor/ghostty/include/ghostty`. The caller sorts them byte-wise so the
/// order matches the regeneration script's `LC_ALL=C sort`, which differs from Rust's
/// component-wise `Path` ordering for paths like `vt.h` next to `vt/key.h`.
fn headers(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            headers(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("h") {
            out.push(path);
        }
    }
}

#[test]
fn generated_bindings_match_vendored_headers() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = crate_dir.parent().unwrap().parent().unwrap();
    let ffi = fs::read_to_string(crate_dir.join("src/ghostty/ffi.rs")).unwrap();
    let recorded = ffi
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("// header-sha256: "))
        .expect("fingerprint line");

    let mut paths = Vec::new();
    headers(&root.join("vendor/ghostty/include/ghostty"), &mut paths);
    paths.sort_by(|a, b| {
        a.as_os_str()
            .as_encoded_bytes()
            .cmp(b.as_os_str().as_encoded_bytes())
    });
    let mut hasher = Sha256::new();
    for path in paths {
        hasher.update(fs::read(path).unwrap());
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        actual, recorded,
        "vendor/ghostty headers changed; run scripts/regen-ghostty-bindings.sh"
    );
}
