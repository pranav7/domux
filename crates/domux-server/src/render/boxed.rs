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
    /// The area inside the border, without drawing anything. `render` returns this, and the
    /// pointer hit test asks for it: a cell hits the grid cell the reader sees under it only
    /// while the two agree about where the border is.
    pub fn inner_of(area: Rect) -> Rect {
        Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        )
    }

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
        // The title is program-controlled - a pane's title is whatever it emitted with OSC 0
        // or OSC 2 - so sanitize before measuring, or the budget is computed from cells that
        // will not be drawn. `put` sanitizes too, but only this side knows the budget, and a
        // budget measured on undrawable text truncates in the wrong place. The flag is
        // domux's own word, sanitized only so both sides measure by one rule.
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
        // An empty title draws nothing at all: the two pads of " {title} " would blank two
        // cells of rule and leave a gap in the border.
        if budget > 0 && area.width >= 4 && !title.is_empty() {
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
        Self::inner_of(area)
    }
}

/// Writes `text` from `x`, one grapheme per cell, wide graphemes taking two, clipping at the
/// buffer's right edge.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) -> u16 {
    // Saturating, not `- 1`: a client reports its own size and nothing clamps it, so a
    // zero-width buffer reaches here and the subtraction would panic.
    let last_x = (buf.area.x + buf.area.width).saturating_sub(1);
    put_within(buf, x, y, last_x, text, style)
}

/// `put`, but clipping at `last_x` inclusive as well as at the buffer. A box passes its own
/// right edge here so that no arithmetic mistake upstream can write into a neighbouring box:
/// budgets keep text inside the border, and this keeps a wrong budget from escaping it.
pub fn put_within(buf: &mut Buffer, x: u16, y: u16, last_x: u16, text: &str, style: Style) -> u16 {
    use unicode_segmentation::UnicodeSegmentation;
    let mut cx = x;
    let max_x = (buf.area.x + buf.area.width).min(last_x.saturating_add(1));
    for g in text.graphemes(true) {
        // Every caller draws program-controlled text somewhere - a pane title, a pasted tab
        // name, a prompt, a path - so the primitive refuses what a terminal cannot draw
        // rather than trusting each caller to sanitize first. A control character would be
        // flushed to the terminal verbatim and move the cursor; a zero-width grapheme would
        // take a cell here while measuring none, and walk text out of its box.
        if g.chars().any(char::is_control) || display_width(g) == 0 {
            continue;
        }
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
