//! Copy mode: the scrollback offset a pane snapshots at, and the selection over it. Task 19
//! writes the real thing; until then this carries only the offset the pane runtime reads.

pub struct CopyMode {
    /// Lines above the live screen's top row that the pane snapshots at. 0 is the live screen.
    pub offset: usize,
}
