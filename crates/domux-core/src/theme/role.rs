//! The roles: the public names a theme sets and rendering reads. Adding a role is fine.
//! Renaming or removing one breaks theme files, so it needs a decision record.

use super::guard::{DOT_OFF_FLOOR, FLOOR, GROUND_FLOOR, LINE_FLOOR, RULE_FLOOR};

/// Declares every role once: its variant, its public name, what it colours, how the readability
/// guards treat it, and the grounds it is held to its floor on. The order here is the order of
/// `Role::ALL` and of a theme's values.
macro_rules! roles {
    ($( #[doc = $what:literal] $variant:ident = $name:literal, $guard:ident on [$($on:ident),* $(,)?]; )*) => {
        /// One thing the chrome colours.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Role {
            $( #[doc = $what] $variant, )*
        }

        impl Role {
            /// Every role, in the order a theme holds them.
            pub const ALL: &'static [Role] = &[$(Role::$variant),*];

            /// The public snake_case name a theme file writes.
            pub fn name(self) -> &'static str {
                match self {
                    $(Role::$variant => $name,)*
                }
            }

            /// What the role colours, in one line.
            pub fn what(self) -> &'static str {
                match self {
                    $(Role::$variant => $what.trim_ascii(),)*
                }
            }

            /// How the readability guards treat the role.
            pub fn guard(self) -> Guard {
                match self {
                    $(Role::$variant => Guard::$guard,)*
                }
            }

            /// The grounds the role is drawn on and held to its floor against. For the top bar,
            /// the toast and the selected row, the ground they keep a step from.
            pub fn grounds(self) -> &'static [Role] {
                match self {
                    $(Role::$variant => &[$(Role::$on),*],)*
                }
            }

            /// The role a public name names, if any.
            pub fn from_name(name: &str) -> Option<Role> {
                match name {
                    $($name => Some(Role::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

/// How the readability guards treat a role. They run only for a theme that read the terminal's
/// answers, and `theme::guard` says how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// A role other roles are drawn on. The top bar, the toast and the selected row keep a step
    /// from the overlay background.
    Ground,
    /// A text tier. The tiers written as a blend move together, so they keep their order.
    TextTier,
    /// A colour, held to the floor on its own.
    Colour,
    /// A colour that says something is wrong or waiting. A palette slot is used for it only when
    /// it is red.
    Red,
    /// A colour that says something is well. A palette slot is used for it only when it is green.
    Green,
    /// A kind colour or a band end, held to the floor on its own. It is never hue tested.
    Kind,
    /// A line, or the stay awake dot for "not held": held on its own to a floor lower than text's.
    Line,
    /// Never moved.
    Unguarded,
}

roles! {
    // Grounds
    /// Every cell of an overlay: the switcher, the agents overlay, Keys, the name box and the confirmation.
    OverlayBackground = "overlay_background", Ground on [];
    /// The top bar's row.
    TopBarBackground = "top_bar_background", Ground on [OverlayBackground];
    /// The toast's cells.
    ToastBackground = "toast_background", Ground on [OverlayBackground];
    /// The selected row.
    Fill = "fill", Ground on [OverlayBackground];
    /// The sidebar column, its boxes and its hint row.
    SidebarBackground = "sidebar_background", Ground on [];
    /// The tab row on the panes.
    TabRowBackground = "tab_row_background", Ground on [];

    // Lines
    /// The rule under a project header, and the bar between tabs and the tab row's elision.
    Rule = "rule", Line on [
        OverlayBackground,
        SidebarBackground,
        TopBarBackground,
        TabRowBackground,
    ];
    /// The dot and the arrow between words in the top bar, the hint row, the footer and an agent row.
    Separator = "separator", Line on [
        OverlayBackground,
        SidebarBackground,
        TopBarBackground,
        TabRowBackground,
    ];
    /// An unfocused box's border and flag, the new tab plus, and the toast's border.
    Border = "border", Line on [
        OverlayBackground,
        SidebarBackground,
        TopBarBackground,
        TabRowBackground,
        ToastBackground,
    ];

    // Text
    /// Ordinary text.
    Text = "text", TextTier on [
        OverlayBackground,
        TopBarBackground,
        ToastBackground,
        SidebarBackground,
        TabRowBackground,
    ];
    /// The clock, the toast's later lines, and the Keys legend.
    SoftText = "soft_text", TextTier on [
        OverlayBackground,
        TopBarBackground,
        ToastBackground,
        SidebarBackground,
        TabRowBackground,
    ];
    /// An unfocused box's title, other tabs, project headers and the place in an agent row.
    DimText = "dim_text", TextTier on [
        OverlayBackground,
        TopBarBackground,
        ToastBackground,
        SidebarBackground,
        TabRowBackground,
    ];
    /// Words in the hint row, the footer and the top bar's right end, and empty text.
    FaintText = "faint_text", TextTier on [
        OverlayBackground,
        TopBarBackground,
        ToastBackground,
        SidebarBackground,
        TabRowBackground,
    ];
    /// Text on the accent fill: the current tab while keys go to a pane, and the tab name prompt.
    OnAccent = "on_accent", Unguarded on [];
    /// Text on a pill.
    OnPill = "on_pill", Unguarded on [];

    // Colours
    /// The focused region's border, bold title and flag, and the accent fill.
    Accent = "accent", Colour on [
        OverlayBackground,
        TopBarBackground,
        SidebarBackground,
        TabRowBackground,
    ];
    /// A key in the top bar, the hint row, the footer, Keys, the name box, the confirmation and copy mode.
    HintKey = "hint_key", Colour on [
        OverlayBackground,
        TopBarBackground,
        SidebarBackground,
        TabRowBackground,
    ];
    /// A named or live workspace's name.
    WorkspaceName = "workspace_name", Colour on [OverlayBackground, Fill, SidebarBackground];
    /// A branch name.
    Branch = "branch", Colour on [OverlayBackground, SidebarBackground];
    /// An open pull request.
    PrOpen = "pr_open", Green on [OverlayBackground, SidebarBackground];
    /// A merged pull request.
    PrMerged = "pr_merged", Colour on [OverlayBackground, SidebarBackground];
    /// A closed pull request.
    PrClosed = "pr_closed", Red on [OverlayBackground, SidebarBackground];
    /// An ok pill's ground.
    PillOk = "pill_ok", Green on [OverlayBackground, SidebarBackground];
    /// A refused pill's ground.
    PillError = "pill_error", Red on [OverlayBackground, SidebarBackground];
    /// The config error in the top bar.
    ConfigError = "config_error", Red on [OverlayBackground, TopBarBackground, TabRowBackground];
    /// A confirmation's question.
    Question = "question", Red on [OverlayBackground];
    /// The dot while an agent is waiting.
    WaitingDot = "waiting_dot", Red on [OverlayBackground, Fill, SidebarBackground];
    /// The stay awake dot while the hold is on.
    StayAwakeDotOn = "stay_awake_dot_on", Green on [
        OverlayBackground,
        TopBarBackground,
        TabRowBackground,
    ];
    /// The stay awake dot while the hold is off.
    StayAwakeDotOff = "stay_awake_dot_off", Line on [
        OverlayBackground,
        TopBarBackground,
        TabRowBackground,
    ];
    /// A recap on a working, waiting, compacting or unseen row.
    Recap = "recap", TextTier on [
        OverlayBackground,
        TopBarBackground,
        ToastBackground,
        SidebarBackground,
        TabRowBackground,
    ];
    /// A recap on a seen row.
    RecapSeen = "recap_seen", TextTier on [
        OverlayBackground,
        TopBarBackground,
        ToastBackground,
        SidebarBackground,
        TabRowBackground,
    ];

    // Kinds and the band
    /// A Claude agent's kind colour.
    Claude = "claude", Kind on [OverlayBackground, SidebarBackground];
    /// A Codex agent's kind colour.
    Codex = "codex", Kind on [OverlayBackground, SidebarBackground];
    /// An OpenCode agent's kind colour.
    Opencode = "opencode", Kind on [OverlayBackground, SidebarBackground];
    /// The compacting arrow half way through its breath, between the compacting band's two ends.
    Compacting = "compacting", Kind on [OverlayBackground, SidebarBackground];
    /// The dim end of the band on a Claude agent's working word.
    BandClaudeDim = "band_claude_dim", Kind on [OverlayBackground, SidebarBackground];
    /// The bright end of the band on a Claude agent's working word.
    BandClaudeBright = "band_claude_bright", Kind on [OverlayBackground, SidebarBackground];
    /// The dim end of the band on a Codex agent's working word.
    BandCodexDim = "band_codex_dim", Kind on [OverlayBackground, SidebarBackground];
    /// The bright end of the band on a Codex agent's working word.
    BandCodexBright = "band_codex_bright", Kind on [OverlayBackground, SidebarBackground];
    /// The dim end of the band on an OpenCode agent's working word.
    BandOpencodeDim = "band_opencode_dim", Kind on [OverlayBackground, SidebarBackground];
    /// The bright end of the band on an OpenCode agent's working word.
    BandOpencodeBright = "band_opencode_bright", Kind on [OverlayBackground, SidebarBackground];
    /// The dim end of the band on the compacting word, and of the compacting arrow's breath.
    BandCompactingDim = "band_compacting_dim", Kind on [OverlayBackground, SidebarBackground];
    /// The bright end of the band on the compacting word, and of the compacting arrow's breath.
    BandCompactingBright = "band_compacting_bright", Kind on [OverlayBackground, SidebarBackground];
}

impl Role {
    /// How many roles a theme holds.
    pub const COUNT: usize = Role::ALL.len();

    /// The contrast the readability guards hold the role to on each of its grounds, or `None`
    /// for a role they never guard. For a ground, the step it keeps from the overlay background.
    pub fn floor(self) -> Option<f64> {
        match self.guard() {
            Guard::Ground => Some(GROUND_FLOOR),
            Guard::TextTier | Guard::Colour | Guard::Red | Guard::Green | Guard::Kind => {
                Some(FLOOR)
            }
            Guard::Line => Some(match self {
                Role::Rule => RULE_FLOOR,
                Role::StayAwakeDotOff => DOT_OFF_FLOOR,
                _ => LINE_FLOOR,
            }),
            Guard::Unguarded => None,
        }
    }

    /// True for a role other roles are drawn on.
    pub fn is_ground(self) -> bool {
        self.guard() == Guard::Ground
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_role_has_one_public_name_and_it_round_trips() {
        assert_eq!(Role::ALL.len(), 43);
        let names: BTreeSet<&str> = Role::ALL.iter().map(|r| r.name()).collect();
        assert_eq!(names.len(), 43, "two roles share a name");
        for (i, role) in Role::ALL.iter().enumerate() {
            assert_eq!(*role as usize, i, "{role:?} is out of order");
            let name = role.name();
            assert!(
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name} is not snake_case"
            );
            assert_eq!(Role::from_name(name), Some(*role));
            assert!(
                !role.what().is_empty(),
                "{name} does not say what it colours"
            );
        }
        assert_eq!(Role::from_name("acent"), None);
        assert_eq!(Role::from_name("Accent"), None);
        let grounds: Vec<&str> = Role::ALL
            .iter()
            .filter(|r| r.is_ground())
            .map(|r| r.name())
            .collect();
        assert_eq!(
            grounds,
            [
                "overlay_background",
                "top_bar_background",
                "toast_background",
                "fill",
                "sidebar_background",
                "tab_row_background"
            ]
        );
    }

    /// The readability table, written out: each role's guard, its floor and the grounds it is
    /// held on.
    #[test]
    fn every_guarded_role_is_held_against_the_grounds_the_readability_table_names() {
        use super::super::guard::{DOT_OFF_FLOOR, FLOOR, GROUND_FLOOR, LINE_FLOOR, RULE_FLOOR};
        use Guard::*;
        use Role::*;
        const TIER: &[Role] = &[
            OverlayBackground,
            TopBarBackground,
            ToastBackground,
            SidebarBackground,
            TabRowBackground,
        ];
        const KEYS: &[Role] = &[
            OverlayBackground,
            TopBarBackground,
            SidebarBackground,
            TabRowBackground,
        ];
        const ROW: &[Role] = &[OverlayBackground, Fill, SidebarBackground];
        const LIST: &[Role] = &[OverlayBackground, SidebarBackground];
        const BAR: &[Role] = &[OverlayBackground, TopBarBackground, TabRowBackground];
        const LINES: &[Role] = &[
            OverlayBackground,
            SidebarBackground,
            TopBarBackground,
            TabRowBackground,
        ];
        const BORDER: &[Role] = &[
            OverlayBackground,
            SidebarBackground,
            TopBarBackground,
            TabRowBackground,
            ToastBackground,
        ];
        let want: &[(Role, Guard, Option<f64>, &[Role])] = &[
            (OverlayBackground, Ground, Some(GROUND_FLOOR), &[]),
            (
                TopBarBackground,
                Ground,
                Some(GROUND_FLOOR),
                &[OverlayBackground],
            ),
            (
                ToastBackground,
                Ground,
                Some(GROUND_FLOOR),
                &[OverlayBackground],
            ),
            (Fill, Ground, Some(GROUND_FLOOR), &[OverlayBackground]),
            (SidebarBackground, Ground, Some(GROUND_FLOOR), &[]),
            (TabRowBackground, Ground, Some(GROUND_FLOOR), &[]),
            (Rule, Line, Some(RULE_FLOOR), LINES),
            (Separator, Line, Some(LINE_FLOOR), LINES),
            (Border, Line, Some(LINE_FLOOR), BORDER),
            (Text, TextTier, Some(FLOOR), TIER),
            (SoftText, TextTier, Some(FLOOR), TIER),
            (DimText, TextTier, Some(FLOOR), TIER),
            (FaintText, TextTier, Some(FLOOR), TIER),
            (OnAccent, Unguarded, None, &[]),
            (OnPill, Unguarded, None, &[]),
            (Accent, Colour, Some(FLOOR), KEYS),
            (HintKey, Colour, Some(FLOOR), KEYS),
            (WorkspaceName, Colour, Some(FLOOR), ROW),
            (Branch, Colour, Some(FLOOR), LIST),
            (PrOpen, Green, Some(FLOOR), LIST),
            (PrMerged, Colour, Some(FLOOR), LIST),
            (PrClosed, Red, Some(FLOOR), LIST),
            (PillOk, Green, Some(FLOOR), LIST),
            (PillError, Red, Some(FLOOR), LIST),
            (ConfigError, Red, Some(FLOOR), BAR),
            (Question, Red, Some(FLOOR), &[OverlayBackground]),
            (WaitingDot, Red, Some(FLOOR), ROW),
            (StayAwakeDotOn, Green, Some(FLOOR), BAR),
            (StayAwakeDotOff, Line, Some(DOT_OFF_FLOOR), BAR),
            (Recap, TextTier, Some(FLOOR), TIER),
            (RecapSeen, TextTier, Some(FLOOR), TIER),
            (Claude, Kind, Some(FLOOR), LIST),
            (Codex, Kind, Some(FLOOR), LIST),
            (Opencode, Kind, Some(FLOOR), LIST),
            (Compacting, Kind, Some(FLOOR), LIST),
            (BandClaudeDim, Kind, Some(FLOOR), LIST),
            (BandClaudeBright, Kind, Some(FLOOR), LIST),
            (BandCodexDim, Kind, Some(FLOOR), LIST),
            (BandCodexBright, Kind, Some(FLOOR), LIST),
            (BandOpencodeDim, Kind, Some(FLOOR), LIST),
            (BandOpencodeBright, Kind, Some(FLOOR), LIST),
            (BandCompactingDim, Kind, Some(FLOOR), LIST),
            (BandCompactingBright, Kind, Some(FLOOR), LIST),
        ];
        assert_eq!(want.len(), Role::COUNT);
        for (role, guard, floor, grounds) in want {
            assert_eq!(role.guard(), *guard, "{}", role.name());
            assert_eq!(role.floor(), *floor, "{}", role.name());
            assert_eq!(role.grounds(), *grounds, "{}", role.name());
            assert!(
                role.grounds().iter().all(|g| g.is_ground()),
                "{} is held on a role that is not a ground",
                role.name()
            );
        }
    }
}
