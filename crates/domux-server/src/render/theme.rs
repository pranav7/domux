//! From a theme's roles to the colours a cell is drawn in. The colours themselves live in the
//! theme (`domux_core::theme`); the built-in `domux` theme holds the values interface spec
//! section 9.1 names.

use domux_core::model::agent::AgentKind;
use domux_core::theme::{Paint, Role, Theme};
use ratatui::style::Color;

/// The colour `theme` gives `role`. A role painted in the terminal's default colour is
/// `Color::Reset`, which is how a cell asks for that colour.
pub fn color(theme: &Theme, role: Role) -> Color {
    match theme.get(role) {
        Paint::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        Paint::Default => Color::Reset,
    }
}

/// The two ends of the band that runs along a working word, dim and bright.
///
/// Channel triples rather than `Color`, because the row draws every value between them:
/// these are numbers to mix, where every other colour is one to set.
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

/// The three colours the compacting arrow breathes through: the dim end, the colour it rests
/// in half way, and the bright end (MUX-48).
///
/// Three rather than a band's two, so the `compacting` role is still what the arrow is drawn
/// in whenever it is neither dimmed nor lit, and a theme that sets that role sees it there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Breath {
    pub dim: (u8, u8, u8),
    pub rest: (u8, u8, u8),
    pub bright: (u8, u8, u8),
}

impl Breath {
    /// The colour `lit` of the way through the breath, which is what `render::shimmer::breath`
    /// answers for a tick: the first half mixes dim into the resting colour and the second
    /// mixes the resting colour into bright.
    pub fn at(&self, lit: f64) -> Color {
        if lit <= 0.5 {
            Shimmer {
                dim: self.dim,
                bright: self.rest,
            }
            .at(lit * 2.0)
        } else {
            Shimmer {
                dim: self.rest,
                bright: self.bright,
            }
            .at(lit * 2.0 - 1.0)
        }
    }
}

/// The colour of an agent's kind: the name standing in for a session name, the kind on line 2
/// and the glyph while it works.
pub fn agent_color(theme: &Theme, kind: AgentKind) -> Color {
    color(theme, kind_role(kind))
}

/// The band that runs along an agent's working word, from the kind's two band roles.
pub fn agent_shimmer(theme: &Theme, kind: AgentKind) -> Shimmer {
    let (dim, bright) = match kind {
        AgentKind::Claude => (Role::BandClaudeDim, Role::BandClaudeBright),
        AgentKind::Codex => (Role::BandCodexDim, Role::BandCodexBright),
        AgentKind::Opencode => (Role::BandOpencodeDim, Role::BandOpencodeBright),
    };
    shimmer(theme, dim, bright)
}

/// The band on the word `Compacting`, whatever the kind.
pub fn compacting_shimmer(theme: &Theme) -> Shimmer {
    shimmer(theme, Role::BandCompactingDim, Role::BandCompactingBright)
}

/// The compacting arrow's breath, whatever the kind: the ends of the band on the word
/// `Compacting`, through the `compacting` role itself.
pub fn compacting_breath(theme: &Theme) -> Breath {
    Breath {
        dim: channels(theme, Role::BandCompactingDim),
        rest: channels(theme, Role::Compacting),
        bright: channels(theme, Role::BandCompactingBright),
    }
}

fn kind_role(kind: AgentKind) -> Role {
    match kind {
        AgentKind::Claude => Role::Claude,
        AgentKind::Codex => Role::Codex,
        AgentKind::Opencode => Role::Opencode,
    }
}

/// A band between two roles.
fn shimmer(theme: &Theme, dim: Role, bright: Role) -> Shimmer {
    Shimmer {
        dim: channels(theme, dim),
        bright: channels(theme, bright),
    }
}

/// A role as channels to mix. A band and a breath are mixed channel by channel, and the
/// terminal's default has no channels to mix, so a role painted `default` takes the `domux`
/// theme's value for it, which is always a hex for the roles that are mixed.
fn channels(theme: &Theme, role: Role) -> (u8, u8, u8) {
    let rgb = match theme.get(role) {
        Paint::Rgb(rgb) => rgb,
        Paint::Default => match Theme::domux().get(role) {
            Paint::Rgb(rgb) => rgb,
            Paint::Default => unreachable!("the domux theme paints every mixed role in hex"),
        },
    };
    (rgb.r, rgb.g, rgb.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A role painted in hex is that hex, and one painted in the terminal's default is the
    /// reset colour, which is how a cell asks the terminal for its own.
    #[test]
    fn a_role_converts_to_its_hex_or_to_reset_when_default() {
        let domux = Theme::domux();
        assert_eq!(
            color(domux, Role::OverlayBackground),
            Color::Rgb(0x1e, 0x1e, 0x2e)
        );
        assert_eq!(color(domux, Role::SidebarBackground), Color::Reset);
        let themed = domux.with(Role::Text, Paint::Default);
        assert_eq!(color(&themed, Role::Text), Color::Reset);
    }

    /// The `domux` theme's kind roles are the manifests' `color_hex`, so the kind reads the
    /// same colour in domux as the agent's own manifest gives it.
    #[test]
    fn the_agent_colours_match_the_manifests() {
        let r = crate::agents::manifests::Registry::builtin();
        for kind in AgentKind::ALL {
            let hex = r.for_kind(kind).unwrap().color_hex.trim_start_matches('#');
            let n = u32::from_str_radix(hex, 16).unwrap();
            let expected = Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
            assert_eq!(agent_color(Theme::domux(), kind), expected, "{kind}");
        }
    }

    /// The two ends of a band are the colours it is drawn between, and nothing in between is
    /// stored: `at` mixes them.
    #[test]
    fn a_shimmer_runs_from_its_dim_end_to_its_bright_end() {
        let band = Shimmer {
            dim: (0xb8, 0x5e, 0x47),
            bright: (0xff, 0xc9, 0xb0),
        };
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

    /// A breath passes through all three of its colours: the dim end, the resting colour half
    /// way, and the bright end at the top, mixing the two it is between everywhere else.
    #[test]
    fn a_breath_runs_from_dim_through_its_rest_to_bright() {
        let breath = Breath {
            dim: (0x6f, 0x6f, 0xcf),
            rest: (0xaf, 0xaf, 0xff),
            bright: (0xd8, 0xd8, 0xff),
        };
        assert_eq!(breath.at(0.0), Color::Rgb(0x6f, 0x6f, 0xcf), "unlit is dim");
        assert_eq!(
            breath.at(0.5),
            Color::Rgb(0xaf, 0xaf, 0xff),
            "half way rests"
        );
        assert_eq!(
            breath.at(1.0),
            Color::Rgb(0xd8, 0xd8, 0xff),
            "fully lit is bright"
        );
        assert_eq!(
            breath.at(0.25),
            Color::Rgb(0x8f, 0x8f, 0xe7),
            "a quarter is half way from dim to rest"
        );
        assert_eq!(
            breath.at(0.75),
            Color::Rgb(0xc4, 0xc4, 0xff),
            "three quarters is half way from rest to bright"
        );
    }

    /// The compacting arrow breathes on the compacting band's two ends, through the
    /// `compacting` role's own colour.
    #[test]
    fn the_compacting_breath_is_drawn_from_its_three_roles() {
        assert_eq!(
            compacting_breath(Theme::domux()),
            Breath {
                dim: (0x6f, 0x6f, 0xcf),
                rest: (0xaf, 0xaf, 0xff),
                bright: (0xd8, 0xd8, 0xff),
            }
        );
    }

    /// Under the `domux` theme every kind has a band of its own, and so has compacting, and
    /// every one of them runs dim to bright on all three channels. A theme file may run a band
    /// either way, so this holds for the built-in theme and not for every theme.
    #[test]
    fn every_kinds_shimmer_runs_dim_to_bright() {
        let bands = AgentKind::ALL
            .iter()
            .map(|kind| (kind.to_string(), agent_shimmer(Theme::domux(), *kind)))
            .chain(std::iter::once((
                "compacting".to_string(),
                compacting_shimmer(Theme::domux()),
            )));
        for (kind, band) in bands {
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
