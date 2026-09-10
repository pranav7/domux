//! The colours of interface spec section 9.1: Catppuccin Mocha, the palette V1 ships, plus
//! the accent, the branch pink and the workspace teal. The last two are the spec's own
//! values and not Mocha's, so read the hexes from the spec's table rather than from a
//! Mocha palette.

use ratatui::style::Color;

pub const BASE: Color = Color::Rgb(0x1e, 0x1e, 0x2e);
pub const MANTLE: Color = Color::Rgb(0x18, 0x18, 0x25);
pub const SURFACE0: Color = Color::Rgb(0x31, 0x32, 0x44);
pub const SURFACE1: Color = Color::Rgb(0x45, 0x47, 0x5a);
pub const SURFACE2: Color = Color::Rgb(0x58, 0x5b, 0x70);
pub const OVERLAY0: Color = Color::Rgb(0x6c, 0x70, 0x86);
pub const OVERLAY1: Color = Color::Rgb(0x7f, 0x84, 0x9c);
pub const SUBTEXT0: Color = Color::Rgb(0xa6, 0xad, 0xc8);
pub const TEXT: Color = Color::Rgb(0xcd, 0xd6, 0xf4);
pub const BLUE: Color = Color::Rgb(0x89, 0xb4, 0xfa);
pub const GREEN: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
pub const RED: Color = Color::Rgb(0xf3, 0x8b, 0xa8);
pub const MAUVE: Color = Color::Rgb(0xcb, 0xa6, 0xf7);
/// The focused region's border and bold title, and the current tab's fill. Nothing else.
/// The domux logo's mauve, the same value as MAUVE (ruled 2026-09-06).
pub const ACCENT: Color = Color::Rgb(0xcb, 0xa6, 0xf7);
/// Branch names.
pub const PINK: Color = Color::Rgb(0xe3, 0xb4, 0xd8);
/// Workspace names.
pub const TEAL: Color = Color::Rgb(0x93, 0xe2, 0xd5);

/// The agent kinds (interface spec 9.1). The manifests carry the same values as hex strings.
pub const CLAUDE: Color = Color::Rgb(0xde, 0x73, 0x56);
pub const CODEX: Color = Color::Rgb(0x89, 0xb4, 0xfa);
pub const OPENCODE: Color = Color::Rgb(0xc6, 0x78, 0xb8);
/// The dot, glyph and word while compacting.
pub const COMPACTING: Color = Color::Rgb(0xaf, 0xaf, 0xff);
/// A recap on a working, waiting, compacting or unseen row.
pub const RECAP: Color = Color::Rgb(0xdd, 0xca, 0xf7);
/// A recap on a seen or exited row: `subtext0`, same as the clock.
pub const RECAP_SEEN: Color = SUBTEXT0;

pub fn agent_color(kind: domux_core::model::agent::AgentKind) -> Color {
    use domux_core::model::agent::AgentKind::*;
    match kind {
        Claude => CLAUDE,
        Codex => CODEX,
        Opencode => OPENCODE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_colours_match_the_manifests() {
        use domux_core::model::agent::AgentKind;
        let r = crate::agents::manifests::Registry::builtin();
        for kind in AgentKind::ALL {
            let hex = r.for_kind(kind).unwrap().color_hex.trim_start_matches('#');
            let n = u32::from_str_radix(hex, 16).unwrap();
            let expected = Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
            assert_eq!(agent_color(kind), expected, "{kind}");
        }
    }
}
