//! The roles: the public names a theme sets and rendering reads. Adding a role is fine.
//! Renaming or removing one breaks theme files, so it needs a decision record.

/// Declares every role once: its variant, its public name, what it colours, and whether other
/// roles are drawn on it. The order here is the order of `Role::ALL` and of a theme's values.
macro_rules! roles {
    ($( #[doc = $what:literal] $variant:ident = $name:literal, ground: $ground:literal; )*) => {
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

            /// True for a role other roles are drawn on.
            pub fn is_ground(self) -> bool {
                match self {
                    $(Role::$variant => $ground,)*
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

roles! {
    // Grounds
    /// Every cell of an overlay: the switcher, the agents overlay, Keys, the name box and the confirmation.
    OverlayBackground = "overlay_background", ground: true;
    /// The top bar's row.
    TopBarBackground = "top_bar_background", ground: true;
    /// The toast's cells.
    ToastBackground = "toast_background", ground: true;
    /// The selected row.
    Fill = "fill", ground: true;
    /// The sidebar column, its boxes and its hint row.
    SidebarBackground = "sidebar_background", ground: true;
    /// The tab row on the panes.
    TabRowBackground = "tab_row_background", ground: true;

    // Lines
    /// The rule under a project header, and the bar between tabs and the tab row's elision.
    Rule = "rule", ground: false;
    /// The dot and the arrow between words in the top bar, the hint row, the footer and an agent row.
    Separator = "separator", ground: false;
    /// An unfocused box's border and flag, the new tab plus, and the toast's border.
    Border = "border", ground: false;

    // Text
    /// Ordinary text.
    Text = "text", ground: false;
    /// The clock, the toast's later lines, and the Keys legend.
    SoftText = "soft_text", ground: false;
    /// An unfocused box's title, other tabs, project headers and the place in an agent row.
    DimText = "dim_text", ground: false;
    /// Words in the hint row, the footer and the top bar's right end, and empty text.
    FaintText = "faint_text", ground: false;
    /// Text on the accent fill: the current tab while keys go to a pane, and the tab name prompt.
    OnAccent = "on_accent", ground: false;
    /// Text on a pill.
    OnPill = "on_pill", ground: false;

    // Colours
    /// The focused region's border, bold title and flag, and the accent fill.
    Accent = "accent", ground: false;
    /// A key in the top bar, the hint row, the footer, Keys, the name box, the confirmation and copy mode.
    HintKey = "hint_key", ground: false;
    /// A named or live workspace's name.
    WorkspaceName = "workspace_name", ground: false;
    /// A branch name.
    Branch = "branch", ground: false;
    /// An open pull request.
    PrOpen = "pr_open", ground: false;
    /// A merged pull request.
    PrMerged = "pr_merged", ground: false;
    /// A closed pull request.
    PrClosed = "pr_closed", ground: false;
    /// An ok pill's ground.
    PillOk = "pill_ok", ground: false;
    /// A refused pill's ground.
    PillError = "pill_error", ground: false;
    /// The config error in the top bar.
    ConfigError = "config_error", ground: false;
    /// A confirmation's question.
    Question = "question", ground: false;
    /// The dot while an agent is waiting.
    WaitingDot = "waiting_dot", ground: false;
    /// The stay awake dot while the hold is on.
    StayAwakeDotOn = "stay_awake_dot_on", ground: false;
    /// The stay awake dot while the hold is off.
    StayAwakeDotOff = "stay_awake_dot_off", ground: false;
    /// A recap on a working, waiting, compacting or unseen row.
    Recap = "recap", ground: false;
    /// A recap on a seen row.
    RecapSeen = "recap_seen", ground: false;

    // Kinds and the band
    /// A Claude agent's kind colour.
    Claude = "claude", ground: false;
    /// A Codex agent's kind colour.
    Codex = "codex", ground: false;
    /// An OpenCode agent's kind colour.
    Opencode = "opencode", ground: false;
    /// The glyph and the word while compacting.
    Compacting = "compacting", ground: false;
    /// The dim end of the band on a Claude agent's working word.
    BandClaudeDim = "band_claude_dim", ground: false;
    /// The bright end of the band on a Claude agent's working word.
    BandClaudeBright = "band_claude_bright", ground: false;
    /// The dim end of the band on a Codex agent's working word.
    BandCodexDim = "band_codex_dim", ground: false;
    /// The bright end of the band on a Codex agent's working word.
    BandCodexBright = "band_codex_bright", ground: false;
    /// The dim end of the band on an OpenCode agent's working word.
    BandOpencodeDim = "band_opencode_dim", ground: false;
    /// The bright end of the band on an OpenCode agent's working word.
    BandOpencodeBright = "band_opencode_bright", ground: false;
    /// The dim end of the band on the compacting word.
    BandCompactingDim = "band_compacting_dim", ground: false;
    /// The bright end of the band on the compacting word.
    BandCompactingBright = "band_compacting_bright", ground: false;
}

impl Role {
    /// How many roles a theme holds.
    pub const COUNT: usize = Role::ALL.len();
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
}
