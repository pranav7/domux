//! The principle 14 box: a single-line border, the title one cell in from the left corner,
//! an optional flag at the right end, the accent and a bold title when focused.

use crate::render::theme;
use domux_core::text::{display_width, truncate_with_ellipsis};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

pub struct Boxed<'a> {
    pub title: &'a str,
    pub flag: Option<&'a str>,
    pub focused: bool,
}

impl Boxed<'_> {
    /// Draws the box and returns the inner area. Areas under 2x2 draw what fits.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Rect {
        if area.width == 0 || area.height == 0 {
            return Rect::new(area.x, area.y, 0, 0);
        }
        let border = Style::default().fg(if self.focused {
            theme::ACCENT
        } else {
            theme::SURFACE2
        });
        let title_style = if self.focused {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::OVERLAY1)
        };
        let flag_style = Style::default().fg(if self.focused {
            theme::ACCENT
        } else {
            theme::SURFACE2
        });
        let right = area.x + area.width - 1;
        let bottom = area.y + area.height - 1;
        for x in area.x..=right {
            buf[(x, area.y)].set_symbol("─").set_style(border);
            if area.height > 1 {
                buf[(x, bottom)].set_symbol("─").set_style(border);
            }
        }
        for y in area.y..=bottom {
            buf[(area.x, y)].set_symbol("│").set_style(border);
            if area.width > 1 {
                buf[(right, y)].set_symbol("│").set_style(border);
            }
        }
        buf[(area.x, area.y)].set_symbol("┌").set_style(border);
        if area.width > 1 {
            buf[(right, area.y)].set_symbol("┐").set_style(border);
        }
        if area.height > 1 {
            buf[(area.x, bottom)].set_symbol("└").set_style(border);
            if area.width > 1 {
                buf[(right, bottom)].set_symbol("┘").set_style(border);
            }
        }
        // Title: " title " starting one cell in. Flag: " flag " ending one cell before the corner.
        let mut budget = area.width.saturating_sub(4) as usize; // corners plus the two title pads
        let flag_cells = self.flag.map(|f| display_width(f) + 2).unwrap_or(0);
        if flag_cells > 0 && budget > flag_cells {
            budget -= flag_cells;
            let flag = format!(" {} ", self.flag.unwrap_or(""));
            let start = right as usize - 1 - (flag_cells - 1);
            put(buf, start as u16, area.y, &flag, flag_style);
        }
        if budget > 0 && area.width >= 4 {
            let title = truncate_with_ellipsis(self.title, budget);
            put(buf, area.x + 1, area.y, &format!(" {title} "), title_style);
        }
        Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        )
    }
}

/// Writes `text` from `x`, one grapheme per cell, wide graphemes taking two.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) -> u16 {
    use unicode_segmentation::UnicodeSegmentation;
    let mut cx = x;
    let max_x = buf.area.x + buf.area.width;
    for g in text.graphemes(true) {
        let w = display_width(g).max(1) as u16;
        if cx + w > max_x {
            break;
        }
        buf[(cx, y)].set_symbol(g).set_style(style);
        for i in 1..w {
            buf[(cx + i, y)].reset();
            buf[(cx + i, y)].set_style(style);
        }
        cx += w;
    }
    cx
}
