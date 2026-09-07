//! Copy mode: the scrollback offset a pane snapshots at, and the selection over it. Task 19
//! writes the keys and the selection; this is the state the pane runtime and the renderer read.

use crate::render::pane_box::Selection;
use domux_term::{ScrollbackPos, Size};

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
