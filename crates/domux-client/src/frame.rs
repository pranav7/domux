//! Writes server frames to the outer terminal through ratatui's crossterm backend.

use domux_core::proto::{CursorState, FrameDiff, WireColor};
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
        let mut changed: Vec<(u16, u16)> = Vec::with_capacity(diff.cells.len());
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
            if u.x < visible.width && u.y < visible.height {
                changed.push((u.x, u.y));
            }
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
    use ratatui::backend::TestBackend;

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
}
