//! The version string `domux --version` prints.

use std::sync::OnceLock;

/// The workspace version from Cargo.toml, for example `1.0.0`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `git describe --tags --always --dirty` at build time, the release tag when the pipeline built
/// the binary, or `unknown` when neither was available.
pub const GIT_DESCRIBE: &str = env!("DOMUX_GIT_DESCRIBE");

/// `1.0.0` when the build is the tagged release `v1.0.0`; otherwise `1.0.0 (v1.0.0-3-g1a2b3c4)`.
pub fn format_version(version: &str, describe: &str) -> String {
    if describe == format!("v{version}") {
        version.to_string()
    } else {
        format!("{version} ({describe})")
    }
}

/// The version string for clap, computed once and leaked for the `'static` lifetime clap wants.
pub fn long_version() -> &'static str {
    static LONG: OnceLock<String> = OnceLock::new();
    LONG.get_or_init(|| format_version(VERSION, GIT_DESCRIBE))
}

#[cfg(test)]
mod tests {
    use super::format_version;

    #[test]
    fn format_version_is_bare_when_describe_matches_the_tag() {
        assert_eq!(format_version("1.0.0", "v1.0.0"), "1.0.0");
        assert_eq!(
            format_version("1.0.0-beta.1", "v1.0.0-beta.1"),
            "1.0.0-beta.1"
        );
    }

    #[test]
    fn format_version_appends_describe_when_it_differs() {
        assert_eq!(
            format_version("1.0.0", "v1.0.0-3-g1a2b3c4"),
            "1.0.0 (v1.0.0-3-g1a2b3c4)"
        );
        assert_eq!(
            format_version("1.0.0", "v1.0.0-dirty"),
            "1.0.0 (v1.0.0-dirty)"
        );
        assert_eq!(format_version("1.0.0", "unknown"), "1.0.0 (unknown)");
    }
}
