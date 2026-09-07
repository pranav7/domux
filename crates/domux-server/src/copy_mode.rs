//! Copy mode: the scrollback offset a pane snapshots at, and the selection over it. Task 19
//! writes the keys and the selection; this is the state the pane runtime and the renderer read.

use crate::render::pane_box::Selection;

pub struct CopyMode {
    /// Lines above the live screen's top row that the pane snapshots at. 0 is the live screen.
    pub offset: usize,
    /// The copy cursor in grid coordinates (row, col). It replaces the emulator's cursor
    /// while copy mode is on.
    pub cursor: (u16, u16),
}

impl CopyMode {
    pub fn selection(&self) -> Option<Selection> {
        None
    }
}
