//! Themes: the chrome's colours by role. Rendering reads roles, never constants.
//!
//! A theme is a chain of layers that ends at the built-in `domux` theme, which is the colours
//! domux drew before themes.

pub mod answers;
pub mod builtin;
pub mod choice;
pub mod color;
pub mod file;
pub mod role;
pub mod value;

pub use answers::{auto, Desktop, TerminalColors};
pub use choice::ThemeChoice;
pub use color::{contrast, luminance, mix};
pub use file::ThemeLayer;
pub use role::Role;
pub use value::{ColorValue, ValueError};

use domux_term::Rgb;
use std::sync::OnceLock;

/// What a role is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Paint {
    Rgb(Rgb),
    /// The terminal's default colour.
    Default,
}

/// A complete theme: one paint per role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    paints: [Paint; Role::COUNT],
}

impl Theme {
    /// The paint for one role.
    pub fn get(&self, role: Role) -> Paint {
        self.paints[role as usize]
    }

    /// This theme with one role changed.
    pub fn with(&self, role: Role, paint: Paint) -> Theme {
        let mut theme = self.clone();
        theme.paints[role as usize] = paint;
        theme
    }

    /// The built-in `domux` theme, the colours domux drew before themes.
    pub fn domux() -> &'static Theme {
        static DOMUX: OnceLock<Theme> = OnceLock::new();
        DOMUX.get_or_init(|| {
            let (_, text) = builtin::BUILTIN
                .iter()
                .find(|(name, _)| *name == "domux")
                .expect("domux is built in");
            let (layer, _) = ThemeLayer::parse("domux (built in)", text)
                .unwrap_or_else(|e| panic!("the built-in domux theme: {}", e.0));
            // The root of every chain: its unit tests hold every role to hex or default.
            let paint = |role: Role| match layer.roles.get(&role) {
                Some(ColorValue::Hex(rgb)) => Paint::Rgb(*rgb),
                Some(ColorValue::Default) => Paint::Default,
                other => panic!("the built-in domux theme sets {} to {other:?}", role.name()),
            };
            Theme {
                paints: std::array::from_fn(|i| paint(Role::ALL[i])),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn hex(v: u32) -> Paint {
        Paint::Rgb(Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        })
    }

    /// Written out rather than read from `builtin/domux.toml`, so a change to the file shows
    /// up here as a failure instead of being carried into the expectation.
    #[test]
    fn the_domux_theme_is_the_colours_domux_drew_before_themes() {
        use Role::*;
        let want = [
            (OverlayBackground, hex(0x1e1e2e)),
            (TopBarBackground, hex(0x181825)),
            (ToastBackground, hex(0x181825)),
            (Fill, hex(0x313244)),
            (SidebarBackground, Paint::Default),
            (TabRowBackground, Paint::Default),
            (Rule, hex(0x313244)),
            (Separator, hex(0x45475a)),
            (Border, hex(0x585b70)),
            (Text, hex(0xcdd6f4)),
            (SoftText, hex(0xa6adc8)),
            (DimText, hex(0x7f849c)),
            (FaintText, hex(0x6c7086)),
            (OnAccent, hex(0x1e1e2e)),
            (OnPill, hex(0x1e1e2e)),
            (Accent, hex(0xcba6f7)),
            (HintKey, hex(0x89b4fa)),
            (WorkspaceName, hex(0x93e2d5)),
            (Branch, hex(0xe3b4d8)),
            (PrOpen, hex(0xa6e3a1)),
            (PrMerged, hex(0xcba6f7)),
            (PrClosed, hex(0xf38ba8)),
            (PillOk, hex(0xa6e3a1)),
            (PillError, hex(0xf38ba8)),
            (ConfigError, hex(0xf38ba8)),
            (Question, hex(0xf38ba8)),
            (WaitingDot, hex(0xf38ba8)),
            (StayAwakeDotOn, hex(0xa6e3a1)),
            (StayAwakeDotOff, hex(0x585b70)),
            (Recap, hex(0xddcaf7)),
            (RecapSeen, hex(0xa6adc8)),
            (Claude, hex(0xde7356)),
            (Codex, hex(0x89b4fa)),
            (Opencode, hex(0xc678b8)),
            (Compacting, hex(0xafafff)),
            (BandClaudeDim, hex(0xb85e47)),
            (BandClaudeBright, hex(0xffc9b0)),
            (BandCodexDim, hex(0x6478a8)),
            (BandCodexBright, hex(0xc8daff)),
            (BandOpencodeDim, hex(0x9f5d93)),
            (BandOpencodeBright, hex(0xf0b5e3)),
            (BandCompactingDim, hex(0x6f6fcf)),
            (BandCompactingBright, hex(0xd8d8ff)),
        ];
        assert_eq!(want.len(), Role::ALL.len());
        let theme = Theme::domux();
        for (role, paint) in want {
            assert_eq!(theme.get(role), paint, "{}", role.name());
        }
    }

    #[test]
    fn the_domux_theme_sets_the_two_terminal_grounds_to_default_and_every_other_role_in_hex() {
        let (_, text) = builtin::BUILTIN
            .iter()
            .find(|(name, _)| *name == "domux")
            .expect("domux is built in");
        let (layer, warnings) = ThemeLayer::parse("domux (built in)", text).expect("parses");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(layer.extends, None);
        for role in Role::ALL {
            let value = layer.roles.get(role);
            match role {
                Role::SidebarBackground | Role::TabRowBackground => {
                    assert_eq!(value, Some(&ColorValue::Default), "{}", role.name())
                }
                _ => assert!(
                    matches!(value, Some(ColorValue::Hex(_))),
                    "{} is {value:?}",
                    role.name()
                ),
            }
        }
    }

    #[test]
    fn every_built_in_theme_parses_without_a_warning() {
        assert!(!builtin::BUILTIN.is_empty());
        for (name, text) in builtin::BUILTIN {
            let (layer, warnings) = ThemeLayer::parse(&format!("{name} (built in)"), text)
                .unwrap_or_else(|e| panic!("{name}: {}", e.0));
            assert!(warnings.is_empty(), "{name}: {warnings:?}");
            assert!(!layer.roles.is_empty(), "{name} sets no role");
        }
    }

    #[test]
    fn a_theme_with_one_role_changed_differs_in_that_role_only() {
        let base = Theme::domux();
        let paint = hex(0x123456);
        let changed = base.with(Role::HintKey, paint);
        for role in Role::ALL {
            if *role == Role::HintKey {
                assert_eq!(changed.get(*role), paint);
                assert_ne!(base.get(*role), paint);
            } else {
                assert_eq!(changed.get(*role), base.get(*role), "{}", role.name());
            }
        }
    }
}
