//! The attach protocol. Task 9 fills this file; the Model needs `Capabilities` first.

use domux_term::Rgb;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the client's outer terminal can do, negotiated at attach.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    pub truecolor: bool,
    pub kitty_keyboard: bool,
    pub hyperlinks: bool,
    /// The outer terminal accepts OSC 52 for the clipboard.
    pub osc52: bool,
    /// The outer terminal's default colours when it answered OSC 10 and 11.
    pub default_fg: Option<Rgb>,
    pub default_bg: Option<Rgb>,
}
