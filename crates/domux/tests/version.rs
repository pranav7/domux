//! `domux --version` prints the binary name, the Cargo version and the git describe.

use std::process::Command;

#[test]
fn version_flag_prints_name_version_and_describe() {
    let out = Command::new(env!("CARGO_BIN_EXE_domux"))
        .arg("--version")
        .output()
        .expect("run domux --version");
    assert!(out.status.success(), "exit status {:?}", out.status);
    assert!(
        out.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let version = env!("CARGO_PKG_VERSION");
    // Set by crates/domux/build.rs for every target of this package, tests included.
    let describe = env!("DOMUX_GIT_DESCRIBE");
    let want = if describe == format!("v{version}") {
        format!("{} {version}\n", domux_core::names::BIN_NAME)
    } else {
        format!("{} {version} ({describe})\n", domux_core::names::BIN_NAME)
    };
    assert_eq!(stdout, want);
}
