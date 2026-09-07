//! The principle 14 box: a single-line border, the title one cell in from the left corner,
//! an optional flag at the right end, the accent and a bold title when focused.

use crate::render::theme;
use domux_core::text::{display_width, sanitize_for_display, truncate_with_ellipsis};
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
        //
        // Both strings are program-controlled - a pane's title is whatever it emitted with
        // OSC 0 or OSC 2 - so sanitize before measuring. Without it a control character
        // measures one cell, passes every bound and is then flushed into the terminal, and a
        // zero-width grapheme measures none, satisfies any budget and still takes a cell when
        // drawn. After sanitizing, a measured cell is a drawn cell.
        let title = sanitize_for_display(self.title);
        let flag = self.flag.map(sanitize_for_display);
        let mut budget = area.width.saturating_sub(4) as usize; // corners plus the two title pads
        let flag_cells = flag.as_deref().map(|f| display_width(f) + 2).unwrap_or(0);
        // `budget > flag_cells` rather than `budget >= flag_cells + 1`: clippy's int_plus_one
        // rejects the second and they are the same predicate on usize.
        if flag_cells > 0 && budget > flag_cells {
            budget -= flag_cells;
            let text = format!(" {} ", flag.as_deref().unwrap_or(""));
            let start = right as usize - 1 - (flag_cells - 1);
            put_within(buf, start as u16, area.y, right, &text, flag_style);
        }
        if budget > 0 && area.width >= 4 {
            let text = truncate_with_ellipsis(&title, budget);
            put_within(
                buf,
                area.x + 1,
                area.y,
                right,
                &format!(" {text} "),
                title_style,
            );
        }
        Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        )
    }
}

/// Writes `text` from `x`, one grapheme per cell, wide graphemes taking two, clipping at the
/// buffer's right edge.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) -> u16 {
    put_within(buf, x, y, buf.area.x + buf.area.width - 1, text, style)
}

/// `put`, but clipping at `last_x` inclusive as well as at the buffer. A box passes its own
/// right edge here so that no arithmetic mistake upstream can write into a neighbouring box:
/// budgets keep text inside the border, and this keeps a wrong budget from escaping it.
pub fn put_within(buf: &mut Buffer, x: u16, y: u16, last_x: u16, text: &str, style: Style) -> u16 {
    use unicode_segmentation::UnicodeSegmentation;
    let mut cx = x;
    let max_x = (buf.area.x + buf.area.width).min(last_x.saturating_add(1));
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
