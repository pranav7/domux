//! Copy mode: the scrollback offset a pane snapshots at, and the selection over it. Task 19
//! writes the keys and the selection; this is the state the pane runtime and the renderer read.

use crate::pane::PaneRuntime;
use crate::render::pane_box::Selection;
use domux_term::{Key, KeyEvent, ScrollbackPos, Size};

pub struct CopyMode {
    /// Lines above the live screen's top row that the pane snapshots at. 0 is the live screen.
    pub offset: usize,
    /// The copy cursor in grid coordinates (row, col). It replaces the emulator's cursor
    /// while copy mode is on.
    pub cursor: (u16, u16),
    /// The pane's screen when copy mode opened, so the cursor can be kept on it.
    pub size: Size,
    /// Where a selection started, once one has. `None` means nothing is selected yet.
    pub anchor: Option<ScrollbackPos>,
}

impl CopyMode {
    /// Copy mode as it opens: on the live screen, at the pane's own cursor, nothing selected.
    pub fn new(size: Size, cursor: (u16, u16)) -> CopyMode {
        CopyMode {
            offset: 0,
            cursor,
            size,
            anchor: None,
        }
    }

    pub fn selection(&self) -> Option<Selection> {
        None
    }
}

/// What one key in copy mode did. `input::route_key` acts on this, so Task 19 can rewrite
/// `handle_key` without touching the router.
pub enum CopyOutcome {
    /// Copy mode stays open.
    Continue,
    /// Text was yanked: it goes to the pressing client's clipboard and copy mode closes.
    Copy(String),
    /// Copy mode closes with nothing copied.
    Leave,
}

/// Task 19 writes the movement, the selection and the yank. Until then Esc leaves and every
/// other key is swallowed, so a pane in copy mode never passes keys to a program that is
/// not reading them.
pub fn handle_key(_pane: &mut PaneRuntime, key: &KeyEvent) -> CopyOutcome {
    if key.key == Key::Escape {
        CopyOutcome::Leave
    } else {
        CopyOutcome::Continue
    }
}
