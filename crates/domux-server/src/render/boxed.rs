//! The principle 14 box: a single-line border, the title one cell in from the left corner,
//! an optional flag at the right end, the accent and a bold title when focused.

use crate::render::theme::color;
use domux_core::text::{display_width, sanitize_for_display, truncate_with_ellipsis};
use domux_core::theme::{Role, Theme};
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
    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) -> Rect {
        self.border(theme, area, buf, false);
        self.labels(theme, area, buf)
    }

    /// Draws the box's border, and with `join` meets a line already drawn under it rather
    /// than covering it: a corner on a neighbour's edge becomes `├`, `┬`, `┴` or `┤`, and four
    /// corners meeting become `┼` (decision record 0064). Pane boxes join, because neighbours
    /// share their facing edge, and so does the sidebar's edge beside them. Nothing drawn over
    /// another surface joins: an overlay's corner on a pane's text is a corner.
    ///
    /// On a line that was already there a box that is not focused keeps the colour it found,
    /// so a shared edge wears the accent when either box beside it has the keys, whatever
    /// order they are drawn in.
    pub fn border(&self, theme: &Theme, area: Rect, buf: &mut Buffer, join: bool) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let border = Style::default().fg(if self.focused {
            color(theme, Role::Accent)
        } else {
            color(theme, Role::Border)
        });
        let right = area.x + area.width - 1;
        let bottom = area.y + area.height - 1;
        let closed = area.height > 1;
        let wide = area.width > 1;
        let mut set = |x: u16, y: u16, arms: u8| {
            let cell = &mut buf[(x, y)];
            let under = if join { arms_of(cell.symbol()) } else { 0 };
            cell.set_symbol(glyph(arms | under));
            if under == 0 || self.focused {
                cell.set_style(border);
            }
        };
        // Corners first in each arm's reach, so a box one cell wide or tall still draws the
        // corners it always drew: `┌` and `┐` on a box one row tall.
        for x in area.x + 1..right {
            set(x, area.y, LEFT | RIGHT);
            if closed {
                set(x, bottom, LEFT | RIGHT);
            }
        }
        for y in area.y + 1..bottom {
            set(area.x, y, UP | DOWN);
            if wide {
                set(right, y, UP | DOWN);
            }
        }
        set(area.x, area.y, DOWN | RIGHT);
        if wide {
            set(right, area.y, DOWN | LEFT);
        }
        if closed {
            set(area.x, bottom, UP | RIGHT);
            if wide {
                set(right, bottom, UP | LEFT);
            }
        }
    }

    /// Draws the title and the flag on the top border, and returns the inner area. After every
    /// border a frame joins, so a neighbour's border drawn later never covers a title that sits
    /// on the line the two share.
    pub fn labels(&self, theme: &Theme, area: Rect, buf: &mut Buffer) -> Rect {
        if area.width == 0 || area.height == 0 {
            return Rect::new(area.x, area.y, 0, 0);
        }
        let title_style = if self.focused {
            Style::default()
                .fg(color(theme, Role::Accent))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color(theme, Role::DimText))
        };
        let flag_style = Style::default().fg(if self.focused {
            color(theme, Role::Accent)
        } else {
            color(theme, Role::Border)
        });
        let right = area.x + area.width - 1;
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

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// The box-drawing glyphs by the arms they reach out with, indexed by `UP | DOWN | LEFT |
/// RIGHT`. A cell with one arm or none is drawn as the line it lies on, which is what a box
/// one cell wide or tall draws.
const GLYPHS: [&str; 16] = [
    "─", "│", "│", "│", "─", "┘", "┐", "┤", "─", "└", "┌", "├", "─", "┴", "┬", "┼",
];

fn glyph(arms: u8) -> &'static str {
    GLYPHS[usize::from(arms & 0b1111)]
}

/// The arms of a box-drawing glyph already in a cell, or 0 for anything else.
fn arms_of(symbol: &str) -> u8 {
    match symbol {
        "─" => LEFT | RIGHT,
        "│" => UP | DOWN,
        "┌" => DOWN | RIGHT,
        "┐" => DOWN | LEFT,
        "└" => UP | RIGHT,
        "┘" => UP | LEFT,
        "├" => UP | DOWN | RIGHT,
        "┤" => UP | DOWN | LEFT,
        "┬" => DOWN | LEFT | RIGHT,
        "┴" => UP | LEFT | RIGHT,
        "┼" => UP | DOWN | LEFT | RIGHT,
        _ => 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(buf: &Buffer) -> Vec<String> {
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    fn plain(focused: bool) -> Boxed<'static> {
        Boxed {
            title: "",
            flag: None,
            focused,
        }
    }

    /// Two boxes side by side over a third below them, each pair sharing a line, draw one line
    /// between each pair and a junction wherever lines meet (decision record 0064).
    #[test]
    fn boxes_that_share_a_line_join_where_they_meet() {
        let theme = Theme::domux();
        let mut buf = Buffer::empty(Rect::new(0, 0, 7, 5));
        plain(false).border(theme, Rect::new(0, 0, 4, 3), &mut buf, true);
        plain(false).border(theme, Rect::new(3, 0, 4, 3), &mut buf, true);
        plain(false).border(theme, Rect::new(0, 2, 7, 3), &mut buf, true);
        assert_eq!(
            rows(&buf),
            vec!["┌──┬──┐", "│  │  │", "├──┴──┤", "│     │", "└─────┘"]
        );
    }

    /// A box that is not focused keeps the colour it finds on a line it shares, so the accent
    /// of a focused neighbour survives whichever is drawn last.
    #[test]
    fn a_shared_line_keeps_the_accent_of_a_focused_box() {
        let theme = Theme::domux();
        let mut buf = Buffer::empty(Rect::new(0, 0, 7, 3));
        plain(true).border(theme, Rect::new(0, 0, 4, 3), &mut buf, true);
        plain(false).border(theme, Rect::new(3, 0, 4, 3), &mut buf, true);
        assert_eq!(buf[(3, 1)].fg, color(theme, Role::Accent));
        assert_eq!(buf[(6, 1)].fg, color(theme, Role::Border));
    }

    /// Without `join` a box covers what is under it, which is what an overlay over a pane's
    /// text needs: a `│` in the text must not turn its corner into a junction.
    #[test]
    fn a_box_that_does_not_join_covers_the_line_under_it() {
        let theme = Theme::domux();
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 3));
        buf[(0, 0)].set_symbol("│");
        plain(false).render(theme, Rect::new(0, 0, 4, 3), &mut buf);
        assert_eq!(rows(&buf), vec!["┌──┐", "│  │", "└──┘"]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        plain(false).render(theme, Rect::new(0, 0, 4, 1), &mut buf);
        assert_eq!(
            rows(&buf),
            vec!["┌──┐"],
            "a box one row tall keeps its corners"
        );
    }
}
