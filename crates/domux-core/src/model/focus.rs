//! The one focus target each client has, and the overlay it may have open.

use crate::ids::{PaneId, ProjectId, TabId, WorkspaceId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a client's keys go. `Pane` mirrors the tab's focused pane; `Region` is a domux
/// region that handles keys itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Focus {
    Pane(PaneId),
    Region(RegionKind),
}

/// The region kinds of V2.0 (roadmap section 5.3). `Switcher` is the Projects box inside
/// the switcher; `AgentsOverlay` is the Agents box inside the agents overlay; `Overlay` is
/// any other overlay (prompt, confirmation, usage, help). M1 uses `Overlay`; M2 uses the
/// switcher and sidebar kinds; M3 uses `AgentsOverlay` and `SidebarAgents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    Switcher,
    AgentsOverlay,
    SidebarProjects,
    SidebarAgents,
    Overlay,
}

/// A one-line text input with a caret, for prompts. Pure, so every edit has a unit test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextInput {
    pub text: String,
    /// Caret position in chars.
    pub cursor: usize,
}

impl TextInput {
    /// Starts with `text` and the caret at its end.
    pub fn new(text: impl Into<String>) -> TextInput {
        let text = text.into();
        TextInput {
            cursor: text.chars().count(),
            text,
        }
    }
    pub fn insert(&mut self, c: char) {
        let mut chars: Vec<char> = self.text.chars().collect();
        chars.insert(self.cursor, c);
        self.text = chars.into_iter().collect();
        self.cursor += 1;
    }
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let mut chars: Vec<char> = self.text.chars().collect();
            chars.remove(self.cursor - 1);
            self.text = chars.into_iter().collect();
            self.cursor -= 1;
        }
    }
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }
    pub fn home(&mut self) {
        self.cursor = 0;
    }
    pub fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }
}

/// What a prompt names. M1: a tab (interface spec 4.7, drawn in the tab's own cell).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptKind {
    TabName { tab: TabId, input: TextInput },
}

/// What a confirmation asks about. M2 opens these; M1 only declares them.
///
/// Adjacently tagged, for the reason `Focus` is: an internally tagged enum cannot represent a
/// newtype variant holding a string, and serde fails at runtime rather than at compile time -
/// the derive builds, and the first attempt to serialize one returns "cannot serialize tagged
/// newtype variant".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ConfirmKind {
    DeleteWorkspace(WorkspaceId),
    RemoveProject(ProjectId),
}

/// The overlay a client has open, if any (roadmap section 5.3). Every V2.0 variant is
/// declared here; M1 opens `Prompt` and `Help`, M2 `Switcher`, `NameWorkspace` and
/// `Confirm`, M3 `Agents`, M4 `Usage`.
///
/// Adjacently tagged, like `Focus` and `ConfirmKind`. Internal tagging could not serialize
/// `NameWorkspace` or `Confirm` at all, and it serialized `Prompt` into JSON with two `kind`
/// keys that would not parse back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Overlay {
    Switcher,
    Agents,
    Prompt(PromptKind),
    NameWorkspace(WorkspaceId),
    Confirm(ConfirmKind),
    Usage,
    Help,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ProjectId, TabId, WorkspaceId};

    /// Every `Overlay` a client can hold. The match below is the forcing function: a new variant
    /// makes it non-exhaustive, and whoever adds one has to come here and add it to the list too.
    fn every_overlay() -> Vec<Overlay> {
        let all = vec![
            Overlay::Switcher,
            Overlay::Agents,
            Overlay::Prompt(PromptKind::TabName {
                tab: TabId("t_0001".into()),
                input: TextInput::new("hello"),
            }),
            Overlay::NameWorkspace(WorkspaceId("w_0001".into())),
            Overlay::Confirm(ConfirmKind::DeleteWorkspace(WorkspaceId("w_0001".into()))),
            Overlay::Confirm(ConfirmKind::RemoveProject(ProjectId("p_0001".into()))),
            Overlay::Usage,
            Overlay::Help,
        ];
        for o in &all {
            match o {
                Overlay::Switcher
                | Overlay::Agents
                | Overlay::Prompt(_)
                | Overlay::NameWorkspace(_)
                | Overlay::Confirm(_)
                | Overlay::Usage
                | Overlay::Help => {}
            }
        }
        all
    }

    /// The defect this pins is not a compile error: an internally tagged enum with a newtype
    /// variant holding a string derives cleanly and fails at the first serialize. Under internal
    /// tagging `NameWorkspace` and both `Confirm`s returned "cannot serialize tagged newtype
    /// variant", and `Prompt` produced `{"kind":"prompt","kind":"tab_name",...}` - two `kind`
    /// keys, which came back as "duplicate field `kind`". `ClientView` derives both halves and
    /// holds an `Option<Overlay>`, so every one of them was a round trip waiting to be used.
    #[test]
    fn every_overlay_survives_a_json_round_trip() {
        for o in every_overlay() {
            let s = serde_json::to_string(&o)
                .unwrap_or_else(|e| panic!("could not serialize {o:?}: {e}"));
            let back: Overlay =
                serde_json::from_str(&s).unwrap_or_else(|e| panic!("could not read {s} back: {e}"));
            assert_eq!(back, o, "round trip changed the value: {s}");
        }
    }

    #[test]
    fn text_input_edits_by_char_and_keeps_the_caret_in_range() {
        let mut t = TextInput::new("ab");
        assert_eq!(t.cursor, 2);
        t.insert('c');
        assert_eq!((t.text.as_str(), t.cursor), ("abc", 3));
        t.left();
        t.left();
        t.insert('漢');
        assert_eq!((t.text.as_str(), t.cursor), ("a漢bc", 2));
        t.backspace();
        assert_eq!((t.text.as_str(), t.cursor), ("abc", 1));
        t.home();
        t.backspace();
        assert_eq!((t.text.as_str(), t.cursor), ("abc", 0));
        t.end();
        t.right();
        assert_eq!(t.cursor, 3);
    }
}
