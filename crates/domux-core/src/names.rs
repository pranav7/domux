//! The names that change at the M3 cut-over. Edit this file and the `[[bin]]` name in
//! `crates/domux/Cargo.toml`; nothing else spells `domux2`.

/// The binary and the prefix of every path until the cut-over.
pub const BIN_NAME: &str = "domux2";
/// The product name shown to people. Unchanged by the cut-over.
pub const PRODUCT_NAME: &str = "domux";
pub const STATE_DIR_NAME: &str = "domux2";
pub const CONFIG_DIR_NAME: &str = "domux2";
pub const SOCKET_FILE_NAME: &str = "domux2.sock";
/// The name of the socket directory under `/tmp` when `XDG_RUNTIME_DIR` is unset: `domux2-<uid>`.
pub const SOCKET_DIR_PREFIX: &str = "domux2-";
