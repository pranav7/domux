//! What the terminal and the desktop answered: the colours `terminal` reads, and where the
//! client runs, which is what `auto` asks.

use domux_term::Rgb;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the terminal said its colours are. An absent colour is one the terminal did not
/// answer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct TerminalColors {
    /// The default foreground, from OSC 10.
    pub fg: Option<Rgb>,
    /// The default background, from OSC 11.
    pub bg: Option<Rgb>,
    /// Palette slots 0 to 15, from OSC 4.
    pub palette: [Option<Rgb>; 16],
}

/// The desktop the client runs on, as far as `auto` needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum Desktop {
    #[default]
    Unknown,
    Omarchy,
}

/// The theme `auto` names on a desktop. A per-OS default later is a new `Desktop` variant and
/// a new arm here.
pub fn auto(desktop: Desktop) -> &'static str {
    match desktop {
        Desktop::Omarchy => "terminal",
        Desktop::Unknown => "domux",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_names_terminal_on_omarchy_and_domux_everywhere_else() {
        assert_eq!(auto(Desktop::Omarchy), "terminal");
        assert_eq!(auto(Desktop::Unknown), "domux");
        assert_eq!(auto(Desktop::default()), "domux");
    }

    #[test]
    fn terminal_colors_are_unanswered_by_default() {
        let colors = TerminalColors::default();
        assert_eq!(colors.fg, None);
        assert_eq!(colors.bg, None);
        assert_eq!(colors.palette, [None; 16]);
    }

    #[test]
    fn terminal_colors_and_the_desktop_round_trip_through_bincode_and_json() {
        let rgb = |r, g, b| Some(Rgb { r, g, b });
        let mut palette = [None; 16];
        palette[0] = rgb(1, 2, 3);
        palette[15] = rgb(250, 251, 252);
        let colors = TerminalColors {
            fg: rgb(0xcd, 0xd6, 0xf4),
            bg: None,
            palette,
        };
        let bytes = bincode::serialize(&colors).unwrap();
        assert_eq!(
            bincode::deserialize::<TerminalColors>(&bytes).unwrap(),
            colors
        );
        let json = serde_json::to_string(&colors).unwrap();
        assert_eq!(
            serde_json::from_str::<TerminalColors>(&json).unwrap(),
            colors
        );
        for desktop in [Desktop::Unknown, Desktop::Omarchy] {
            let bytes = bincode::serialize(&desktop).unwrap();
            assert_eq!(bincode::deserialize::<Desktop>(&bytes).unwrap(), desktop);
        }
    }

    #[test]
    fn the_schema_gives_the_palette_sixteen_slots() {
        let schema = serde_json::to_value(schemars::schema_for!(TerminalColors)).unwrap();
        let palette = &schema["properties"]["palette"];
        assert_eq!(palette["minItems"], 16, "{palette}");
        assert_eq!(palette["maxItems"], 16, "{palette}");
        assert!(schemars::schema_for!(Desktop).as_value().is_object());
    }
}
