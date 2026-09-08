//! Copy mode: a mode of one pane. Movement walks the scrollback, `v` starts a selection,
//! Enter copies it to the client's clipboard and leaves, Esc leaves.

use crate::pane::PaneRuntime;
use crate::render::pane_box::Selection;
use crate::render::theme;
use crate::render::top_bar::Piece;
use crate::render::RenderInput;
use domux_term::{Emulator, Key, KeyEvent, Mode, Mods, ScrollbackPos, Size};
use ratatui::style::Style;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyMode {
    /// Lines the viewport is scrolled above the live screen. 0 is the live screen.
    pub offset: usize,
    /// The copy cursor in viewport coordinates (row, col). It replaces the emulator's cursor
    /// while copy mode is on.
    pub cursor: (u16, u16),
    /// The pane's screen, kept in step with the pane's own size.
    pub size: Size,
    /// Where `v` was pressed, in absolute scrollback coordinates. `None` means nothing is
    /// selected yet.
    pub anchor: Option<ScrollbackPos>,
}

/// What one key in copy mode did.
pub enum CopyOutcome {
    /// Copy mode stays open.
    Continue,
    /// Text was yanked: it goes to the pressing client's clipboard and copy mode closes.
    Copy(String),
    /// Copy mode closes with nothing copied.
    Leave,
    /// Copy mode closes with nothing copied, and this line says why. Enter is the key a user
    /// presses expecting a clipboard, so the two ways it can hand back nothing - a selection
    /// with no text in it, and a read that failed - each say so rather than looking like a
    /// copy that worked (principles 8 and 9).
    NotCopied(&'static str),
}

/// The scrollback rows copy mode may walk.
pub fn history(rt: &PaneRuntime) -> usize {
    rows_to_walk(
        rt.emulator.mode_active(Mode::AltScreen),
        rt.emulator.scrollback_len(),
    )
}

/// The gate: the alternate screen keeps no history of its own, and the primary screen's history
/// is not its to walk while it is up.
///
/// It is `mode_active` that says so and never `scrollback_len() == 0`: the count is 0 for the
/// whole time the alternate screen is up and it is also 0 when the emulator cannot answer, so
/// reading the count as the question would make a full-screen editor - exactly where copy mode
/// has to work - indistinguishable from a failure.
///
/// A function rather than a branch inside `history` because it is the only part of this a test
/// can hold. The emulator answers 0 either way, so no end-to-end test can tell the gate from its
/// absence, and a cleanup that inlined `scrollback_len()` here would stay green - except that it
/// would leave this unused, and an unused function is not green.
fn rows_to_walk(alt_screen: bool, scrollback_len: usize) -> usize {
    if alt_screen {
        0
    } else {
        scrollback_len
    }
}

impl CopyMode {
    /// Copy mode as it opens: on the live screen, at the pane's own cursor, nothing selected.
    pub fn new(size: Size, cursor: (u16, u16)) -> CopyMode {
        CopyMode {
            offset: 0,
            cursor: clamp_cursor(cursor, size),
            size,
            anchor: None,
        }
    }

    /// Follows the pane's screen when it changes size, so the copy cursor stays on the screen
    /// and `$` keeps meaning the last column that exists.
    pub fn resized(&mut self, size: Size) {
        self.size = size;
        self.cursor = clamp_cursor(self.cursor, size);
    }

    /// The cursor as an absolute position: `scrollback_len` is the first visible row's
    /// absolute row when unscrolled.
    ///
    /// Saturating, because the history can shrink under an open copy mode - a program
    /// switching to the alternate screen takes it to none - and a row that wrapped would
    /// read from the far end of the scrollback rather than from the screen.
    pub fn abs_cursor(&self, scrollback_len: usize) -> ScrollbackPos {
        ScrollbackPos {
            row: scrollback_len.saturating_sub(self.offset) + self.cursor.0 as usize,
            col: self.cursor.1,
        }
    }

    /// The selection in viewport coordinates, clipped to the visible rows, or `None` when
    /// nothing is selected or the selection is entirely off the screen.
    pub fn selection_at(&self, scrollback_len: usize) -> Option<Selection> {
        let anchor = self.anchor?;
        let (rows, cols) = (self.size.rows, self.size.cols);
        if rows == 0 || cols == 0 {
            return None;
        }
        let (start, end) = ordered(anchor, self.abs_cursor(scrollback_len));
        let top = scrollback_len.saturating_sub(self.offset);
        let bottom = top + rows as usize - 1;
        // A guard, not a case the keys can reach: the copy cursor is one of the two endpoints
        // and it is always on the screen. It keeps the casts below from ever running on a row
        // outside the viewport.
        if end.row < top || start.row > bottom {
            return None;
        }
        // A selection is linear, not rectangular: the part of it that is on the screen starts
        // at the top left when it began above the viewport and ends at the bottom right when
        // it runs past it.
        Some(Selection {
            start: if start.row < top {
                (0, 0)
            } else {
                ((start.row - top) as u16, start.col)
            },
            end: if end.row > bottom {
                (rows - 1, cols - 1)
            } else {
                ((end.row - top) as u16, end.col)
            },
        })
    }
}

/// Moves the copy viewport by a wheel step. Positive lines move into history and open copy
/// mode when needed. Negative lines move towards the live screen and do nothing unless copy
/// mode is already open. Unlike cursor movement, a wheel step changes the viewport at once.
/// Returns whether the gesture belongs to this pane.
pub fn scroll(rt: &mut PaneRuntime, lines: i16) -> bool {
    if lines == 0 {
        return false;
    }
    let history = history(rt);
    if history == 0 || (lines < 0 && rt.copy.is_none()) {
        return false;
    }
    if rt.copy.is_none() {
        let cursor = rt.emulator.cursor();
        rt.copy = Some(CopyMode::new(rt.emulator.size(), (cursor.row, cursor.col)));
    }
    let copy = rt.copy.as_mut().expect("copy mode was opened");
    copy.offset = if lines > 0 {
        copy.offset.saturating_add(lines as usize).min(history)
    } else {
        copy.offset.saturating_sub(lines.unsigned_abs() as usize)
    };
    rt.dirty = true;
    true
}

fn clamp_cursor(cursor: (u16, u16), size: Size) -> (u16, u16) {
    (
        cursor.0.min(size.rows.saturating_sub(1)),
        cursor.1.min(size.cols.saturating_sub(1)),
    )
}

/// The two positions in reading order.
///
/// `text_in_range` swaps its own endpoints, so a reversed range reads the same text either
/// way and a reversed range is not a signal that anything is wrong. The clipping above has to
/// know which end is which regardless, so the order is settled here and nothing depends on
/// the emulator's swap.
fn ordered(a: ScrollbackPos, b: ScrollbackPos) -> (ScrollbackPos, ScrollbackPos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Handles one key while the pane is in copy mode.
pub fn handle_key(rt: &mut PaneRuntime, key: &KeyEvent) -> CopyOutcome {
    let history = history(rt);
    let Some(copy) = rt.copy.as_mut() else {
        return CopyOutcome::Leave;
    };
    // The history can shrink under an open copy mode, so the offset is clamped before it is
    // read as well as after it is moved.
    copy.offset = copy.offset.min(history);
    let rows = copy.size.rows.max(1);
    let cols = copy.size.cols.max(1);
    // Half a screen, and never nothing: a one-row pane would otherwise take C-u and C-d and
    // do nothing at all.
    let half = (rows / 2).max(1) as i64;
    let ctrl = key.mods.contains(Mods::CTRL);
    match (key.key, ctrl) {
        (Key::Escape, _) | (Key::Char('q'), false) => return CopyOutcome::Leave,
        (Key::Enter, _) => {
            let Some(anchor) = copy.anchor else {
                return CopyOutcome::Leave;
            };
            let (start, end) = ordered(anchor, copy.abs_cursor(history));
            return match rt.emulator.text_in_range(start, end) {
                Some(text) if !text.is_empty() => CopyOutcome::Copy(text),
                Some(_) => CopyOutcome::NotCopied("nothing to copy: the selection is blank"),
                None => CopyOutcome::NotCopied("copy failed: the pane could not be read"),
            };
        }
        (Key::Char('v'), false) => copy.anchor = Some(copy.abs_cursor(history)),
        (Key::Char('h'), false) | (Key::Left, _) => copy.cursor.1 = copy.cursor.1.saturating_sub(1),
        (Key::Char('l'), false) | (Key::Right, _) => {
            copy.cursor.1 = (copy.cursor.1 + 1).min(cols - 1)
        }
        (Key::Char('0'), false) => copy.cursor.1 = 0,
        (Key::Char('$'), false) => copy.cursor.1 = cols - 1,
        (Key::Char('j'), false) | (Key::Down, _) => move_rows(copy, 1, rows),
        (Key::Char('k'), false) | (Key::Up, _) => move_rows(copy, -1, rows),
        (Key::Char('d'), true) => move_rows(copy, half, rows),
        (Key::Char('u'), true) => move_rows(copy, -half, rows),
        (Key::PageDown, _) => move_rows(copy, rows as i64, rows),
        (Key::PageUp, _) => move_rows(copy, -(rows as i64), rows),
        (Key::Char('g'), false) => {
            copy.offset = history;
            copy.cursor.0 = 0;
        }
        (Key::Char('G'), false) => {
            copy.offset = 0;
            copy.cursor.0 = rows - 1;
        }
        // Every other key is swallowed: a pane in copy mode never passes keys to a program
        // that is not reading them.
        _ => return CopyOutcome::Continue,
    }
    copy.offset = copy.offset.min(history);
    rt.dirty = true;
    CopyOutcome::Continue
}

/// Moves the cursor by `delta` rows, scrolling the viewport when it would leave the screen.
fn move_rows(copy: &mut CopyMode, delta: i64, rows: u16) {
    let mut row = copy.cursor.0 as i64 + delta;
    if row < 0 {
        copy.offset = copy.offset.saturating_add((-row) as usize);
        row = 0;
    } else if row >= rows as i64 {
        let over = (row - rows as i64 + 1) as usize;
        copy.offset = copy.offset.saturating_sub(over);
        row = rows as i64 - 1;
    }
    copy.cursor.0 = row as u16;
}

/// The clock's place while the pane the keys go to is in copy mode.
///
/// The keys these offer are the keys `route_key` will hand that pane, so the pane is the one
/// `RenderInput::focused_pane` names and never the `Focus::Pane` payload: hints render the
/// bindings that are live (principle 3).
pub fn hint_pieces(input: &RenderInput) -> Option<Vec<Piece>> {
    let rt = input.panes.get(input.focused_pane()?)?;
    let copy = rt.copy.as_ref()?;
    let key = Style::default().fg(theme::BLUE);
    let word = Style::default().fg(theme::OVERLAY0);
    let sep = Style::default().fg(theme::SURFACE1);
    let mut pieces = Vec::new();
    if copy.anchor.is_none() {
        pieces.push(Piece::new("v", key));
        pieces.push(Piece::new(" select", word));
        pieces.push(Piece::new(" · ", sep));
    }
    pieces.push(Piece::new("⏎", key));
    pieces.push(Piece::new(" copy", word));
    pieces.push(Piece::new(" · ", sep));
    pieces.push(Piece::new("esc", key));
    pieces.push(Piece::new(" leave", word));
    Some(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(offset: usize, cursor: (u16, u16), anchor: Option<ScrollbackPos>) -> CopyMode {
        CopyMode {
            offset,
            cursor,
            size: Size { cols: 10, rows: 4 },
            anchor,
        }
    }

    #[test]
    fn the_absolute_cursor_counts_from_the_top_of_the_scrollback() {
        // 20 lines of history: the first visible row is row 20, and two rows up the viewport
        // puts the top row at 18.
        assert_eq!(
            mode(0, (2, 5), None).abs_cursor(20),
            ScrollbackPos { row: 22, col: 5 }
        );
        assert_eq!(
            mode(2, (0, 0), None).abs_cursor(20),
            ScrollbackPos { row: 18, col: 0 }
        );
    }

    /// The history can go to none while copy mode is open - a program switching to the
    /// alternate screen does exactly that - and the row it leaves behind has to be a row on
    /// the screen rather than one wrapped around the bottom of the scrollback.
    #[test]
    fn the_absolute_cursor_holds_when_the_history_shrinks_under_it() {
        assert_eq!(
            mode(9, (1, 0), None).abs_cursor(0),
            ScrollbackPos { row: 1, col: 0 }
        );
        assert_eq!(
            mode(9, (1, 0), Some(ScrollbackPos { row: 0, col: 0 })).selection_at(0),
            Some(Selection {
                start: (0, 0),
                end: (1, 0)
            })
        );
    }

    #[test]
    fn a_selection_reads_the_same_dragged_upward_as_downward() {
        let anchor = ScrollbackPos { row: 22, col: 4 };
        let down = mode(0, (3, 2), Some(anchor)).selection_at(20);
        // The same two endpoints with the cursor and the anchor swapped.
        let up = mode(0, (2, 4), Some(ScrollbackPos { row: 23, col: 2 })).selection_at(20);
        assert_eq!(down, up);
        assert_eq!(
            down,
            Some(Selection {
                start: (2, 4),
                end: (3, 2)
            })
        );
    }

    /// The copy cursor is always on the screen, so it is the anchor that leaves it: above the
    /// top once the cursor has walked down from it, below the bottom once the view has
    /// scrolled up past it. Either way the visible part of the selection reaches the edge.
    #[test]
    fn a_selection_that_runs_past_the_viewport_clips_to_it() {
        // Anchored 5 rows above the visible top, cursor on the second visible row.
        assert_eq!(
            mode(0, (1, 3), Some(ScrollbackPos { row: 15, col: 7 })).selection_at(20),
            Some(Selection {
                start: (0, 0),
                end: (1, 3)
            })
        );
        // Anchored on the last live row, then scrolled to the oldest line: the selection runs
        // off the bottom and fills the screen from the cursor down.
        assert_eq!(
            mode(20, (0, 0), Some(ScrollbackPos { row: 23, col: 0 })).selection_at(20),
            Some(Selection {
                start: (0, 0),
                end: (3, 9)
            })
        );
    }

    /// The rows copy mode may walk while a full-screen program has the screen: none, whatever
    /// the emulator is holding for the primary screen underneath it.
    #[test]
    fn no_scrollback_is_walkable_while_the_alternate_screen_is_up() {
        assert_eq!(rows_to_walk(true, 14), 0);
        assert_eq!(rows_to_walk(false, 14), 14);
        assert_eq!(rows_to_walk(false, 0), 0);
    }

    #[test]
    fn a_resize_keeps_the_copy_cursor_on_the_screen() {
        let mut copy = mode(0, (3, 9), None);
        copy.resized(Size { cols: 5, rows: 2 });
        assert_eq!(copy.cursor, (1, 4));
    }
}
