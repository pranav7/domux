//! The built-in themes, as the same TOML a theme file writes. A future built-in is one file
//! under `builtin/` and one line here.

/// Each built-in theme's name and text.
pub const BUILTIN: &[(&str, &str)] = &[("domux", include_str!("builtin/domux.toml"))];
