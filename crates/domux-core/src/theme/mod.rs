//! Themes: the chrome's colours by role. Rendering reads roles, never constants.
//!
//! A theme is a chain of layers that ends at the built-in `domux` theme, which is the colours
//! domux drew before themes.

pub mod answers;
pub mod builtin;
pub mod chain;
pub mod choice;
pub mod color;
pub mod file;
#[cfg(test)]
mod fixture;
pub mod guard;
pub mod role;
#[cfg(test)]
mod themes_md;
pub mod value;

pub use answers::{auto, Desktop, TerminalColors};
pub use chain::{resolve, Chain, Unused};
pub use choice::ThemeChoice;
pub use color::{contrast, luminance, mix, oklch};
pub use file::ThemeLayer;
pub use role::{Guard, Role};
pub use value::{ColorValue, ValueError};

use domux_term::Rgb;
use std::sync::OnceLock;

/// The contrast the background and the foreground need between them before any form reads
/// them. A terminal that says its foreground is its background said nothing useful.
pub const MIN_ANSWER_CONTRAST: f64 = 3.0;

/// What a role is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Paint {
    Rgb(Rgb),
    /// The terminal's default colour.
    Default,
}

/// The theme a config chose, with the chain of its file when it names one that loaded. The
/// server keeps it, and each client paints it against its own terminal and desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Themes {
    pub choice: ThemeChoice,
    /// The chain of a file choice that loaded. `None` for a built-in choice, for `auto`, and for
    /// a file that did not load, which paints what `auto` names.
    pub file: Option<Chain>,
}

impl Default for Themes {
    fn default() -> Themes {
        Themes {
            choice: ThemeChoice::Auto,
            file: None,
        }
    }
}

impl Themes {
    /// The chain this choice paints for a client on `desktop`.
    fn chain(&self, desktop: Desktop) -> &Chain {
        let name = match (&self.choice, &self.file) {
            (ThemeChoice::File(_), Some(chain)) => return chain,
            (ThemeChoice::Builtin(name), _) => name,
            (ThemeChoice::Auto | ThemeChoice::File(_), _) => auto(desktop),
        };
        builtin::chain(name).expect("a built-in choice names a built-in theme")
    }

    /// This client's theme: the chosen chain painted against its terminal's answers.
    pub fn paint(&self, colors: &TerminalColors, desktop: Desktop) -> Theme {
        Theme::paint(self.chain(desktop), colors)
    }

    /// True when this client's theme reads its terminal's answers.
    pub fn reads_terminal(&self, desktop: Desktop) -> bool {
        self.chain(desktop).reads_terminal()
    }
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

    /// Paints a chain against the terminal's answers. For each role, the first layer that sets
    /// it with a value the answers can evaluate wins; `domux` sets every role in hex or
    /// `default`, so every role resolves. A palette slot on a red or green role that is not red
    /// or green counts as a value the answers cannot evaluate.
    ///
    /// The answers count only when the background and the foreground both arrived and their
    /// contrast is at least 3.0. Otherwise every form but hex and `default` is unanswered.
    ///
    /// When some role was read from the answers, the readability guards then move what nobody
    /// chose for this terminal until it is readable on it (`guard`).
    pub fn paint(chain: &Chain, colors: &TerminalColors) -> Theme {
        Theme::paint_traced(chain, colors).0
    }

    /// `paint`, with what the guards did.
    fn paint_traced(chain: &Chain, colors: &TerminalColors) -> (Theme, guard::Trace) {
        Theme::paint_with(chain, colors, true)
    }

    /// `paint` with the guards left out, so a test can see what they moved from.
    #[cfg(test)]
    fn paint_unguarded(chain: &Chain, colors: &TerminalColors) -> Theme {
        Theme::paint_with(chain, colors, false).0
    }

    fn paint_with(chain: &Chain, colors: &TerminalColors, guards: bool) -> (Theme, guard::Trace) {
        let answers = match (colors.bg, colors.fg) {
            (Some(bg), Some(fg)) if contrast(bg, fg) >= MIN_ANSWER_CONTRAST => Some((bg, fg)),
            _ => None,
        };
        let evaluate = |value: &ColorValue| -> Option<Paint> {
            match *value {
                ColorValue::Hex(rgb) => Some(Paint::Rgb(rgb)),
                ColorValue::Default => Some(Paint::Default),
                other => {
                    let (bg, fg) = answers?;
                    let rgb = match other {
                        ColorValue::Background => bg,
                        ColorValue::Foreground => fg,
                        ColorValue::Palette(slot) => colors.palette[usize::from(slot)]?,
                        ColorValue::Blend { n, d } => mix(bg, fg, n, d),
                        ColorValue::Shade { n, d } => {
                            let far = if luminance(bg) < luminance(fg) {
                                Rgb { r: 0, g: 0, b: 0 }
                            } else {
                                Rgb {
                                    r: 255,
                                    g: 255,
                                    b: 255,
                                }
                            };
                            mix(bg, far, n, d)
                        }
                        ColorValue::Hex(_) | ColorValue::Default => unreachable!(),
                    };
                    Some(Paint::Rgb(rgb))
                }
            }
        };
        let layers: Vec<&ThemeLayer> = chain.layers().collect();
        let root = layers.len().saturating_sub(1);
        // The role's value from the layer at `start` on.
        let resolve = |role: Role, start: usize| -> guard::Resolved {
            for (layer, found) in layers.iter().enumerate().skip(start) {
                let Some(value) = found.roles.get(&role) else {
                    continue;
                };
                let Some(paint) = evaluate(value) else {
                    continue;
                };
                if let (ColorValue::Palette(_), Paint::Rgb(rgb)) = (value, paint) {
                    if !guard::passes_hue(role.guard(), rgb) {
                        continue;
                    }
                }
                return guard::Resolved {
                    paint,
                    value: *value,
                    layer,
                };
            }
            // Only a chain whose root does not set every role gets here.
            let paint = Theme::domux().get(role);
            let value = match paint {
                Paint::Rgb(rgb) => ColorValue::Hex(rgb),
                Paint::Default => ColorValue::Default,
            };
            guard::Resolved {
                paint,
                value,
                layer: root,
            }
        };
        let mut resolved: [guard::Resolved; Role::COUNT] =
            std::array::from_fn(|i| resolve(Role::ALL[i], 0));
        let trace = match answers {
            Some((bg, _)) if guards && resolved.iter().any(guard::Resolved::read_answers) => {
                guard::run(&mut resolved, bg, root, resolve)
            }
            _ => guard::Trace::default(),
        };
        let theme = Theme {
            paints: resolved.map(|r| r.paint),
        };
        (theme, trace)
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

    fn rgb(v: u32) -> Rgb {
        Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    /// Ristretto's answers: the background, the foreground and slots 1, 2 and 4.
    fn ristretto() -> TerminalColors {
        let mut palette = [None; 16];
        palette[1] = Some(rgb(0xfd6883));
        palette[2] = Some(rgb(0xadda78));
        palette[4] = Some(rgb(0xf38d70));
        TerminalColors {
            fg: Some(rgb(0xe6d9db)),
            bg: Some(rgb(0x2c2525)),
            palette,
        }
    }

    fn latte() -> TerminalColors {
        TerminalColors {
            fg: Some(rgb(0x4c4f69)),
            bg: Some(rgb(0xeff1f5)),
            palette: [None; 16],
        }
    }

    /// A chain of one file over `domux`.
    fn chain_of(text: &str) -> Chain {
        let (chain, warnings) = resolve(
            &ThemeChoice::File("mine".to_string()),
            builtin::BUILTIN,
            |_| Ok(Some(text.to_string())),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        chain.expect("resolves")
    }

    fn builtin_chain(name: &str) -> &'static Chain {
        builtin::chain(name).expect("built in")
    }

    #[test]
    fn a_hex_theme_paints_as_written_whatever_the_answers() {
        let chain = chain_of(
            "[roles]\naccent = \"#f38d70\"\nfill = \"#413939\"\nsidebar_background = \"#2c2525\"\n",
        );
        let mut answers = vec![TerminalColors::default(), ristretto(), latte()];
        answers.push(TerminalColors {
            fg: Some(rgb(0x000000)),
            bg: None,
            palette: [Some(rgb(0x123456)); 16],
        });
        for colors in &answers {
            let theme = Theme::paint(&chain, colors);
            let want = Theme::domux()
                .with(Role::Accent, hex(0xf38d70))
                .with(Role::Fill, hex(0x413939))
                .with(Role::SidebarBackground, hex(0x2c2525));
            assert_eq!(theme, want, "{colors:?}");
        }
    }

    #[test]
    fn a_blend_in_a_theme_file_is_evaluated_from_the_answers_and_unanswered_without_them() {
        let chain = chain_of("[roles]\nfill = \"blend 1/9\"\n");
        assert_eq!(
            Theme::paint(&chain, &ristretto()).get(Role::Fill),
            hex(0x413939)
        );
        assert_eq!(
            Theme::paint(&chain, &TerminalColors::default()).get(Role::Fill),
            hex(0x313244)
        );
    }

    #[test]
    fn every_form_is_evaluated_from_the_answers() {
        let chain = chain_of(concat!(
            "[roles]\n",
            "overlay_background = \"background\"\n",
            "text = \"foreground\"\n",
            "accent = \"palette 4\"\n",
            "top_bar_background = \"shade 1/5\"\n",
            "on_accent = \"blend 0/9\"\n",
            "on_pill = \"blend 9/9\"\n",
            "tab_row_background = \"default\"\n",
            "question = \"#010203\"\n",
        ));
        let theme = Theme::paint(&chain, &ristretto());
        assert_eq!(theme.get(Role::OverlayBackground), hex(0x2c2525));
        assert_eq!(theme.get(Role::Text), hex(0xe6d9db));
        assert_eq!(theme.get(Role::Accent), hex(0xf38d70));
        assert_eq!(theme.get(Role::OnAccent), hex(0x2c2525));
        assert_eq!(theme.get(Role::OnPill), hex(0xe6d9db));
        assert_eq!(theme.get(Role::TabRowBackground), Paint::Default);
        assert_eq!(theme.get(Role::Question), hex(0x010203));
        // A dark background shades toward black.
        assert_eq!(theme.get(Role::TopBarBackground), hex(0x231e1e));
        // A light one shades toward white. On a role the guards never move, so the shade itself
        // shows.
        let chain = chain_of("[roles]\non_pill = \"shade 1/5\"\n");
        let theme = Theme::paint(&chain, &latte());
        assert_eq!(theme.get(Role::OnPill), hex(0xf2f4f7));
    }

    #[test]
    fn a_palette_slot_that_did_not_arrive_comes_from_the_theme_it_extends() {
        let chain = chain_of("extends = \"terminal\"\n[roles]\naccent = \"palette 12\"\n");
        let theme = Theme::paint(&chain, &ristretto());
        // Slot 12 is unanswered, so terminal's slot 4 is next.
        assert_eq!(theme.get(Role::Accent), hex(0xf38d70));
        // Slot 5 is unanswered and terminal is the last layer that sets branch before domux.
        assert_eq!(theme.get(Role::Branch), hex(0xe3b4d8));
        assert_eq!(theme.get(Role::PrClosed), hex(0xfd6883));
    }

    #[test]
    fn a_terminal_that_answered_nothing_gets_the_domux_theme() {
        let terminal = builtin_chain("terminal");
        assert_eq!(
            &Theme::paint(terminal, &TerminalColors::default()),
            Theme::domux()
        );
    }

    #[test]
    fn a_background_without_a_foreground_gets_the_domux_theme() {
        let terminal = builtin_chain("terminal");
        let mut one_sided = ristretto();
        one_sided.fg = None;
        assert_eq!(&Theme::paint(terminal, &one_sided), Theme::domux());
        let mut one_sided = ristretto();
        one_sided.bg = None;
        assert_eq!(&Theme::paint(terminal, &one_sided), Theme::domux());
    }

    #[test]
    fn a_palette_without_a_background_and_foreground_is_not_used() {
        let terminal = builtin_chain("terminal");
        let colors = TerminalColors {
            fg: None,
            bg: None,
            palette: [Some(rgb(0xfd6883)); 16],
        };
        assert_eq!(&Theme::paint(terminal, &colors), Theme::domux());
    }

    #[test]
    fn a_foreground_too_close_to_its_background_counts_as_no_answer() {
        let terminal = builtin_chain("terminal");
        let mut too_close = ristretto();
        too_close.fg = Some(rgb(0x5a5050));
        assert!(contrast(too_close.fg.unwrap(), too_close.bg.unwrap()) < 3.0);
        assert_eq!(&Theme::paint(terminal, &too_close), Theme::domux());
        let theme = Theme::paint(terminal, &ristretto());
        assert_eq!(theme.get(Role::Text), hex(0xe6d9db));
    }

    #[test]
    fn the_terminal_theme_with_no_answers_paints_the_domux_theme() {
        assert_eq!(
            &Theme::paint(builtin_chain("terminal"), &TerminalColors::default()),
            Theme::domux()
        );
        assert_eq!(
            &Theme::paint(builtin_chain("domux"), &ristretto()),
            Theme::domux()
        );
    }

    #[test]
    fn themes_paint_the_file_the_built_in_or_what_auto_names_for_the_desktop() {
        let terminal_on_ristretto = Theme::paint(builtin_chain("terminal"), &ristretto());
        assert_ne!(&terminal_on_ristretto, Theme::domux());

        let auto = Themes::default();
        assert_eq!(auto.choice, ThemeChoice::Auto);
        assert_eq!(&auto.paint(&ristretto(), Desktop::Unknown), Theme::domux());
        assert_eq!(
            auto.paint(&ristretto(), Desktop::Omarchy),
            terminal_on_ristretto
        );
        assert!(!auto.reads_terminal(Desktop::Unknown));
        assert!(auto.reads_terminal(Desktop::Omarchy));

        let terminal = Themes {
            choice: ThemeChoice::Builtin("terminal"),
            file: None,
        };
        assert_eq!(
            terminal.paint(&ristretto(), Desktop::Unknown),
            terminal_on_ristretto
        );
        assert!(terminal.reads_terminal(Desktop::Unknown));
        let domux = Themes {
            choice: ThemeChoice::Builtin("domux"),
            file: None,
        };
        assert_eq!(&domux.paint(&ristretto(), Desktop::Omarchy), Theme::domux());
        assert!(!domux.reads_terminal(Desktop::Omarchy));

        let loaded = Themes {
            choice: ThemeChoice::File("mine".to_string()),
            file: Some(chain_of("[roles]\nfill = \"blend 1/9\"\n")),
        };
        assert_eq!(
            loaded.paint(&ristretto(), Desktop::Unknown),
            Theme::domux().with(Role::Fill, hex(0x413939))
        );
        assert!(loaded.reads_terminal(Desktop::Unknown));

        // A file that did not load paints what auto names.
        let not_loaded = Themes {
            choice: ThemeChoice::File("mine".to_string()),
            file: None,
        };
        assert_eq!(
            &not_loaded.paint(&ristretto(), Desktop::Unknown),
            Theme::domux()
        );
        assert_eq!(
            not_loaded.paint(&ristretto(), Desktop::Omarchy),
            terminal_on_ristretto
        );
        assert!(not_loaded.reads_terminal(Desktop::Omarchy));
    }

    #[test]
    fn every_built_in_theme_has_a_chain_that_ends_at_domux() {
        for (name, _) in builtin::BUILTIN {
            let chain = builtin::chain(name).expect("built in");
            assert_eq!(chain.names().first(), Some(name));
            assert_eq!(chain.names().last(), Some(&"domux"));
        }
        assert_eq!(builtin::chain("nord"), None);
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
