//! Draws an emulator grid into a ratatui frame buffer.

use domux_term::{Attrs, Color, Cursor, Grid};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color as RColor, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

pub struct PaneWidget<'a> {
    pub grid: &'a Grid,
    pub focused: bool,
    pub border: bool,
}

pub fn to_ratatui_color(c: Color) -> RColor {
    match c {
        Color::Default => RColor::Reset,
        Color::Indexed(i) => RColor::Indexed(i),
        Color::Rgb(rgb) => RColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

/// The cell area inside an optional one-cell border.
pub fn inner_area(area: Rect, border: bool) -> Rect {
    if border {
        Block::default().borders(Borders::ALL).inner(area)
    } else {
        area
    }
}

/// Where the outer terminal's cursor goes for this pane, or `None` when hidden or off-grid.
pub fn cursor_position(area: Rect, border: bool, cursor: &Cursor) -> Option<Position> {
    if !cursor.visible {
        return None;
    }
    let inner = inner_area(area, border);
    if cursor.col >= inner.width || cursor.row >= inner.height {
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
    // ratatui has one underline style; every underline variant renders as it. The
    // decision record lists this as a renderer gap shared by both emulators.
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

impl Widget for PaneWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.border {
            let style = if self.focused {
                Style::default().fg(RColor::Rgb(0x9e, 0xce, 0x6a))
            } else {
                Style::default()
            };
            Block::default()
                .borders(Borders::ALL)
                .border_style(style)
                .render(area, buf);
        }
        let inner = inner_area(area, self.border);
        let size = self.grid.size();
        let rows = size.rows.min(inner.height);
        let cols = size.cols.min(inner.width);
        for r in 0..rows {
            let y = inner.y + r;
            let mut c = 0u16;
            while c < cols {
                let cell = self.grid.cell(r, c);
                let x = inner.x + c;
                if cell.width == 0 {
                    // Right half of a wide grapheme: reset so ratatui's diff skips it.
                    buf[(x, y)].reset();
                    c += 1;
                    continue;
                }
                if cell.width == 2 && c + 1 >= cols {
                    // A wide grapheme that does not fit: draw a blank instead of half a glyph.
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
                target.set_style(style);
                c += cell.width as u16;
            }
        }
    }
}
