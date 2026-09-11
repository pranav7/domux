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
/// The glyph and the word while compacting.
pub const COMPACTING: Color = Color::Rgb(0xaf, 0xaf, 0xff);

/// The two ends of the band that runs along a working word, dim and bright.
///
/// Channel triples rather than `Color`, because the row draws every value between them:
/// these are numbers to mix, where every other colour here is one to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shimmer {
    pub dim: (u8, u8, u8),
    pub bright: (u8, u8, u8),
}

impl Shimmer {
    /// The colour `lit` of the way from the dim end to the bright end, which is what
    /// `render::shimmer::lit` answers for one character.
    pub fn at(&self, lit: f64) -> Color {
        let mix = |a: u8, b: u8| (f64::from(a) * (1.0 - lit) + f64::from(b) * lit).round() as u8;
        Color::Rgb(
            mix(self.dim.0, self.bright.0),
            mix(self.dim.1, self.bright.1),
            mix(self.dim.2, self.bright.2),
        )
    }
}

/// V1's pairs, which it picked by eye rather than deriving from the kind's colour above: kept
/// off the kind's own dim so the characters away from the band fade without going invisible
/// (V1's `claudeShimmerDim` and the three beside it).
pub const SHIMMER_CLAUDE: Shimmer = Shimmer {
    dim: (0xb8, 0x5e, 0x47),
    bright: (0xff, 0xc9, 0xb0),
};
pub const SHIMMER_CODEX: Shimmer = Shimmer {
    dim: (0x64, 0x78, 0xa8),
    bright: (0xc8, 0xda, 0xff),
};
pub const SHIMMER_OPENCODE: Shimmer = Shimmer {
    dim: (0x9f, 0x5d, 0x93),
    bright: (0xf0, 0xb5, 0xe3),
};
pub const SHIMMER_COMPACTING: Shimmer = Shimmer {
    dim: (0x6f, 0x6f, 0xcf),
    bright: (0xd8, 0xd8, 0xff),
};
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

pub fn agent_shimmer(kind: domux_core::model::agent::AgentKind) -> Shimmer {
    use domux_core::model::agent::AgentKind::*;
    match kind {
        Claude => SHIMMER_CLAUDE,
        Codex => SHIMMER_CODEX,
        Opencode => SHIMMER_OPENCODE,
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

    /// The two ends of a band are the colours it is drawn between, and nothing in between is
    /// stored: `at` mixes them.
    #[test]
    fn a_shimmer_runs_from_its_dim_end_to_its_bright_end() {
        let band = SHIMMER_CLAUDE;
        assert_eq!(band.at(0.0), Color::Rgb(0xb8, 0x5e, 0x47), "unlit is dim");
        assert_eq!(
            band.at(1.0),
            Color::Rgb(0xff, 0xc9, 0xb0),
            "fully lit is bright"
        );
        assert_eq!(
            band.at(0.5),
            Color::Rgb(0xdc, 0x94, 0x7c),
            "and half way is half way"
        );
    }

    /// Every kind has one, so no working row falls back to a colour that is not its own, and
    /// every one of them runs dim to bright on all three channels.
    #[test]
    fn every_kinds_shimmer_runs_dim_to_bright() {
        use domux_core::model::agent::AgentKind;
        for kind in AgentKind::ALL.iter().copied() {
            let band = agent_shimmer(kind);
            let ends = [
                (band.dim.0, band.bright.0),
                (band.dim.1, band.bright.1),
                (band.dim.2, band.bright.2),
            ];
            assert!(
                ends.iter().all(|(dim, bright)| bright > dim),
                "{kind} runs dim to bright, not the other way: {band:?}"
            );
        }
    }
}
