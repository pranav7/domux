//! Draws an emulator grid inside a box's inner area, with the copy mode selection reversed.
//! Lifted from crates/m0-spike/src/render.rs.

use domux_term::{Attrs, Color, Cursor, Grid};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color as RColor, Modifier, Style};

/// Inclusive selection in grid coordinates (row, col), start before end in reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: (u16, u16),
    pub end: (u16, u16),
}

impl Selection {
    pub fn contains(&self, row: u16, col: u16) -> bool {
        (row, col) >= self.start && (row, col) <= self.end
    }
}

pub fn to_ratatui_color(c: Color) -> RColor {
    match c {
        Color::Default => RColor::Reset,
        Color::Indexed(i) => RColor::Indexed(i),
        Color::Rgb(rgb) => RColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

pub fn cursor_position(inner: Rect, cursor: &Cursor) -> Option<Position> {
    if !cursor.visible || cursor.col >= inner.width || cursor.row >= inner.height {
        return None;
    }
    Some(Position {
        x: inner.x + cursor.col,
        y: inner.y + cursor.row,
    })
}

fn modifiers(attrs: Attrs) -> Modifier {
    let mut m = Modifier::empty();
    if attrs.contains(Attrs::BOLD) {
        m |= Modifier::BOLD;
    }
    if attrs.contains(Attrs::DIM) {
        m |= Modifier::DIM;
    }
    if attrs.contains(Attrs::ITALIC) {
        m |= Modifier::ITALIC;
    }
    // ratatui has one underline style, so every underline variant renders as it.
    if attrs.intersects(
        Attrs::UNDERLINE
            | Attrs::DOUBLE_UNDERLINE
            | Attrs::CURLY_UNDERLINE
            | Attrs::DOTTED_UNDERLINE
            | Attrs::DASHED_UNDERLINE,
    ) {
        m |= Modifier::UNDERLINED;
    }
    if attrs.contains(Attrs::BLINK) {
        m |= Modifier::SLOW_BLINK;
    }
    if attrs.contains(Attrs::INVERSE) {
        m |= Modifier::REVERSED;
    }
    if attrs.contains(Attrs::INVISIBLE) {
        m |= Modifier::HIDDEN;
    }
    if attrs.contains(Attrs::STRIKETHROUGH) {
        m |= Modifier::CROSSED_OUT;
    }
    m
}

/// Copies the grid into `inner`, clipping to it. Cells inside `selection` get `REVERSED`
/// toggled so the selection reads with or without colour.
pub fn render_grid(grid: &Grid, selection: Option<&Selection>, inner: Rect, buf: &mut Buffer) {
    let size = grid.size();
    let rows = size.rows.min(inner.height);
    let cols = size.cols.min(inner.width);
    for r in 0..rows {
        let y = inner.y + r;
        let mut c = 0u16;
        while c < cols {
            let cell = grid.cell(r, c);
            let x = inner.x + c;
            if cell.width == 0 || (cell.width == 2 && c + 1 >= cols) {
                buf[(x, y)].reset();
                c += 1;
                continue;
            }
            let target = &mut buf[(x, y)];
            target.set_symbol(if cell.text.is_empty() {
                " "
            } else {
                cell.text.as_str()
            });
            let mut style = Style::default()
                .fg(to_ratatui_color(cell.fg))
                .bg(to_ratatui_color(cell.bg))
                .add_modifier(modifiers(cell.attrs));
            if let Some(ul) = cell.underline_color {
                style = style.underline_color(to_ratatui_color(ul));
            }
            if selection.is_some_and(|s| s.contains(r, c)) {
                style = if style.add_modifier.contains(Modifier::REVERSED) {
                    style.remove_modifier(Modifier::REVERSED)
                } else {
                    style.add_modifier(Modifier::REVERSED)
                };
            }
            target.set_style(style);
            if cell.width == 2 {
                buf[(x + 1, y)].reset();
            }
            c += cell.width as u16;
        }
    }
    // Rows and columns beyond the grid (a client larger than the pane's size) stay blank.
    for r in rows..inner.height {
        for cx in 0..inner.width {
            buf[(inner.x + cx, inner.y + r)].reset();
        }
    }
    for r in 0..rows {
        for cx in cols..inner.width {
            buf[(inner.x + cx, inner.y + r)].reset();
        }
    }
}
