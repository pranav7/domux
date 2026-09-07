//! Writes server frames to the outer terminal through ratatui's crossterm backend.

use domux_core::proto::{CursorState, FrameDiff, WireColor};
use domux_core::text::display_width;
use domux_term::CursorShape;
use ratatui::backend::Backend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Color, Modifier};
use std::io::IsTerminal;

/// The client's copy of what the server's screen looks like, and the terminal it is drawn
/// on. The buffer is the server's geometry; the terminal is whatever it is right now, and
/// the two disagree for as long as a resize is in flight.
pub struct Screen {
    pub buffer: Buffer,
    last_shape: Option<(CursorShape, bool)>,
}

impl Screen {
    pub fn new(cols: u16, rows: u16) -> Screen {
        Screen {
            buffer: Buffer::empty(Rect::new(0, 0, cols, rows)),
            last_shape: None,
        }
    }

    /// Draws one frame. A full frame, or one for another geometry, clears first.
    pub fn apply<B: Backend>(&mut self, diff: &FrameDiff, backend: &mut B) -> Result<(), B::Error> {
        if diff.full || self.buffer.area.width != diff.cols || self.buffer.area.height != diff.rows
        {
            self.buffer = Buffer::empty(Rect::new(0, 0, diff.cols, diff.rows));
            backend.clear()?;
        }
        // The terminal can be smaller than the frame: a resize the server has not composed
        // for yet. Cells outside the terminal are kept in the buffer and not drawn. Drawing
        // one would wrap it onto another row, which is worse than leaving it out for the one
        // frame it takes the server to answer the new size with a full frame. A backend that
        // cannot say how big it is gets the frame's own size, which is what it asked for.
        let visible = backend.size().unwrap_or(Size::new(diff.cols, diff.rows));
        for u in &diff.cells {
            if u.x >= diff.cols || u.y >= diff.rows {
                continue;
            }
            let cell: &mut Cell = &mut self.buffer[(u.x, u.y)];
            cell.set_symbol(&u.symbol);
            cell.fg = color(&u.fg);
            cell.bg = color(&u.bg);
            cell.underline_color = u.underline.as_ref().map(color).unwrap_or(Color::Reset);
            cell.modifier = Modifier::from_bits_truncate(u.modifiers);
        }
        // The run is built after every cell is in the buffer, because whether a position may
        // be drawn depends on the symbol its left neighbour ended up with.
        let mut changed: Vec<(u16, u16)> = Vec::with_capacity(diff.cells.len());
        for u in &diff.cells {
            if u.x >= diff.cols || u.y >= diff.rows {
                continue;
            }
            if u.x >= visible.width || u.y >= visible.height {
                continue;
            }
            if self.is_continuation(u.x, u.y) {
                continue;
            }
            changed.push((u.x, u.y));
        }
        backend.hide_cursor()?;
        backend.draw(changed.iter().map(|&(x, y)| (x, y, &self.buffer[(x, y)])))?;
        match &diff.cursor {
            // A cursor outside the terminal cannot be put where it belongs, and putting it
            // anywhere else would say typing lands there. It stays hidden instead.
            Some(CursorState { x, y, shape, blink })
                if *x < visible.width && *y < visible.height =>
            {
                backend.set_cursor_position(Position { x: *x, y: *y })?;
                if self.last_shape != Some((*shape, *blink)) {
                    set_shape(*shape, *blink);
                    self.last_shape = Some((*shape, *blink));
                }
                backend.show_cursor()?;
            }
            _ => {}
        }
        backend.flush()
    }

    /// Whether this position is the second half of a wide grapheme, and so a cell no
    /// terminal ever draws. The server sends one, because its own buffer holds one, and a
    /// backend that is handed it prints the rest of the row a column late: `CrosstermBackend`
    /// omits the `MoveTo` for a position one to the right of the last one it drew, which is
    /// only sound for a run whose cells are each one column wide. The cell stays in the
    /// buffer - it is what the server sent - it is just never part of a run.
    fn is_continuation(&self, x: u16, y: u16) -> bool {
        x > 0 && display_width(self.buffer[(x - 1, y)].symbol()) > 1
    }
}

/// The cursor style is written straight to stdout: it is not part of a cell, so no backend
/// carries it. Nothing is written when stdout is not a terminal, so a test's output stays
/// text. A style that will not write is cosmetic and never fails a frame.
fn set_shape(shape: CursorShape, blink: bool) {
    if !std::io::stdout().is_terminal() {
        return;
    }
    use crossterm::cursor::SetCursorStyle::*;
    let style = match (shape, blink) {
        (CursorShape::Block, true) => BlinkingBlock,
        (CursorShape::Block, false) => SteadyBlock,
        (CursorShape::Underline, true) => BlinkingUnderScore,
        (CursorShape::Underline, false) => SteadyUnderScore,
        (CursorShape::Bar, true) => BlinkingBar,
        (CursorShape::Bar, false) => SteadyBar,
    };
    let _ = crossterm::execute!(std::io::stdout(), style);
}

fn color(c: &WireColor) -> Color {
    match c {
        WireColor::Reset => Color::Reset,
        WireColor::Indexed(i) => Color::Indexed(*i),
        WireColor::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::proto::{CellUpdate, CursorState, FrameDiff, WireColor};
    use ratatui::backend::{CrosstermBackend, TestBackend};

    #[test]
    fn a_full_frame_clears_then_draws_and_a_diff_only_touches_its_cells() {
        let mut backend = TestBackend::new(4, 2);
        let mut screen = Screen::new(4, 2);
        let full = FrameDiff {
            full: true,
            cols: 4,
            rows: 2,
            cells: vec![CellUpdate {
                x: 0,
                y: 0,
                symbol: "a".into(),
                fg: WireColor::Rgb(1, 2, 3),
                bg: WireColor::Reset,
                underline: None,
                modifiers: 0,
            }],
            cursor: Some(CursorState {
                x: 1,
                y: 0,
                shape: domux_term::CursorShape::Bar,
                blink: false,
            }),
        };
        screen.apply(&full, &mut backend).unwrap();
        assert_eq!(backend.buffer()[(0, 0)].symbol(), "a");
        assert_eq!(
            backend.buffer()[(0, 0)].fg,
            ratatui::style::Color::Rgb(1, 2, 3)
        );
        let diff = FrameDiff {
            full: false,
            cols: 4,
            rows: 2,
            cells: vec![CellUpdate {
                x: 3,
                y: 1,
                symbol: "z".into(),
                fg: WireColor::Reset,
                bg: WireColor::Indexed(4),
                underline: None,
                modifiers: ratatui::style::Modifier::BOLD.bits(),
            }],
            cursor: None,
        };
        screen.apply(&diff, &mut backend).unwrap();
        assert_eq!(
            backend.buffer()[(0, 0)].symbol(),
            "a",
            "untouched cells stay"
        );
        assert_eq!(backend.buffer()[(3, 1)].symbol(), "z");
        assert!(backend.buffer()[(3, 1)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD));
    }

    fn cell(x: u16, y: u16, symbol: &str) -> CellUpdate {
        CellUpdate {
            x,
            y,
            symbol: symbol.into(),
            fg: WireColor::Reset,
            bg: WireColor::Reset,
            underline: None,
            modifiers: 0,
        }
    }

    /// The server composed for a terminal that has since shrunk. The cells that no longer
    /// fit are held, not drawn: drawing one would wrap it onto a row that is still correct.
    #[test]
    fn a_frame_bigger_than_the_terminal_draws_only_what_fits() {
        let mut backend = TestBackend::new(4, 2);
        let mut screen = Screen::new(4, 2);
        let diff = FrameDiff {
            full: true,
            cols: 8,
            rows: 4,
            cells: vec![cell(0, 0, "a"), cell(7, 3, "z")],
            cursor: Some(CursorState {
                x: 7,
                y: 3,
                shape: domux_term::CursorShape::Block,
                blink: false,
            }),
        };
        screen.apply(&diff, &mut backend).unwrap();
        assert_eq!(backend.buffer().area, Rect::new(0, 0, 4, 2));
        assert_eq!(backend.buffer()[(0, 0)].symbol(), "a");
        assert_eq!(
            screen.buffer[(7, 3)].symbol(),
            "z",
            "the cell is held for the next full frame"
        );
        assert!(
            !backend.cursor_visible(),
            "a cursor outside the terminal has no honest place, so it stays hidden"
        );
    }

    /// A frame whose geometry the client has already left behind. The buffer follows the
    /// frame and the screen is cleared, so no cell from the old geometry is left standing.
    #[test]
    fn a_frame_for_another_geometry_replaces_the_buffer() {
        let mut backend = TestBackend::new(4, 2);
        let mut screen = Screen::new(4, 2);
        screen
            .apply(
                &FrameDiff {
                    full: true,
                    cols: 4,
                    rows: 2,
                    cells: vec![cell(3, 1, "x")],
                    cursor: None,
                },
                &mut backend,
            )
            .unwrap();
        assert_eq!(backend.buffer()[(3, 1)].symbol(), "x");
        screen
            .apply(
                &FrameDiff {
                    full: false,
                    cols: 2,
                    rows: 1,
                    cells: vec![cell(0, 0, "n")],
                    cursor: None,
                },
                &mut backend,
            )
            .unwrap();
        assert_eq!(screen.buffer.area, Rect::new(0, 0, 2, 1));
        assert_eq!(backend.buffer()[(0, 0)].symbol(), "n");
        assert_eq!(
            backend.buffer()[(3, 1)].symbol(),
            " ",
            "the old geometry was cleared, not left half drawn"
        );
    }

    /// A cell the server placed outside the frame it belongs to. It is skipped rather than
    /// wrapped onto another row or panicked on.
    #[test]
    fn a_cell_outside_the_frame_is_skipped() {
        let mut backend = TestBackend::new(4, 2);
        let mut screen = Screen::new(4, 2);
        screen
            .apply(
                &FrameDiff {
                    full: true,
                    cols: 4,
                    rows: 2,
                    cells: vec![cell(4, 0, "a"), cell(0, 2, "b"), cell(1, 1, "c")],
                    cursor: None,
                },
                &mut backend,
            )
            .unwrap();
        assert_eq!(backend.buffer()[(1, 1)].symbol(), "c");
        backend.assert_buffer_lines(["    ", " c  "]);
    }

    /// The bytes a real terminal would read for one full frame. `TestBackend` records cells
    /// by coordinate, so it cannot see a cursor left in the wrong column; only the escape
    /// sequence can.
    fn bytes_for(
        cols: u16,
        rows: u16,
        cells: Vec<CellUpdate>,
        cursor: Option<CursorState>,
    ) -> String {
        let mut out: Vec<u8> = Vec::new();
        let mut backend = CrosstermBackend::new(&mut out);
        let mut screen = Screen::new(cols, rows);
        screen
            .apply(
                &FrameDiff {
                    full: true,
                    cols,
                    rows,
                    cells,
                    cursor,
                },
                &mut backend,
            )
            .unwrap();
        String::from_utf8(out).unwrap()
    }

    /// What the server sends for a row starting with a wide grapheme: the grapheme, then the
    /// blank continuation cell its own buffer holds, then the rest of the row.
    fn wide_row(row: &[&str]) -> Vec<CellUpdate> {
        row.iter()
            .enumerate()
            .map(|(x, s)| cell(x as u16, 0, s))
            .collect()
    }

    /// The continuation cell is never drawn. Handing it to the backend would print the rest
    /// of the row one column late and wrap its last cell onto the next row.
    #[test]
    fn a_wide_grapheme_repositions_the_run_past_its_continuation_cell() {
        assert_eq!(
            bytes_for(4, 1, wide_row(&["\u{6f22}", " ", "b", "c"]), None),
            "\x1b[2J\x1b[?25l\x1b[1;1H\u{6f22}\x1b[1;3Hbc\x1b[39m\x1b[49m\x1b[59m\x1b[0m"
        );
    }

    #[test]
    fn two_wide_graphemes_in_a_row_each_reposition_the_run() {
        assert_eq!(
            bytes_for(4, 1, wide_row(&["\u{6f22}", " ", "\u{5b57}", " "]), None),
            "\x1b[2J\x1b[?25l\x1b[1;1H\u{6f22}\x1b[1;3H\u{5b57}\x1b[39m\x1b[49m\x1b[59m\x1b[0m"
        );
    }

    /// The continuation cell of a wide grapheme in the last column has nowhere to go but the
    /// next row, so it is the one the terminal would have wrapped.
    #[test]
    fn a_wide_grapheme_at_the_end_of_a_row_leaves_nothing_to_wrap() {
        let bytes = bytes_for(4, 1, wide_row(&["a", "b", "\u{6f22}", " "]), None);
        assert_eq!(
            bytes,
            "\x1b[2J\x1b[?25l\x1b[1;1Hab\u{6f22}\x1b[39m\x1b[49m\x1b[59m\x1b[0m"
        );
    }

    /// The cursor half of a frame, in bytes: hidden while the cells are written, then put
    /// where the server said and shown again.
    #[test]
    fn a_full_frame_clears_hides_the_cursor_and_shows_it_where_the_server_put_it() {
        let bytes = bytes_for(
            4,
            1,
            wide_row(&["a", "b"]),
            Some(CursorState {
                x: 2,
                y: 0,
                shape: domux_term::CursorShape::Block,
                blink: false,
            }),
        );
        assert!(bytes.starts_with("\x1b[2J\x1b[?25l"), "{bytes:?}");
        assert!(bytes.ends_with("\x1b[1;3H\x1b[?25h"), "{bytes:?}");
    }

    /// Two cells with a gap between them are two runs: the backend only omits the `MoveTo`
    /// for a cell one column to the right of the last one it drew.
    #[test]
    fn cells_that_are_not_next_to_each_other_are_repositioned() {
        assert_eq!(
            bytes_for(4, 1, vec![cell(0, 0, "a"), cell(3, 0, "z")], None),
            "\x1b[2J\x1b[?25l\x1b[1;1Ha\x1b[1;4Hz\x1b[39m\x1b[49m\x1b[59m\x1b[0m"
        );
    }
}
