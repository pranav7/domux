//! The built-in themes, as the same TOML a theme file writes. A future built-in is one file
//! under `builtin/` and one line here.

use std::sync::OnceLock;

/// Each built-in theme's name and text.
pub const BUILTIN: &[(&str, &str)] = &[
    ("domux", include_str!("builtin/domux.toml")),
    ("terminal", include_str!("builtin/terminal.toml")),
];

/// The resolved chain of a built-in theme, or `None` for a name that is not built in.
pub fn chain(name: &str) -> Option<&'static super::Chain> {
    static CHAINS: OnceLock<Vec<(&'static str, super::Chain)>> = OnceLock::new();
    let chains = CHAINS.get_or_init(|| {
        BUILTIN
            .iter()
            .map(|(name, _)| {
                let choice =
                    super::ThemeChoice::parse(name).expect("a built-in name is a theme name");
                // A built-in chain reads no file: its unit tests hold it to resolving without one.
                let (chain, warnings) = super::resolve(&choice, BUILTIN, |_| Ok(None));
                let chain =
                    chain.unwrap_or_else(|_| panic!("the built-in theme {name}: {warnings:?}"));
                (*name, chain)
            })
            .collect()
    });
    chains
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, chain)| chain)
}
