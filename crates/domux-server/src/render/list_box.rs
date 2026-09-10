//! The box the Projects rows and, from M3, the Agents rows are drawn in: rows of styled
//! lines, one filled row, and scrolling that keeps the filled row visible. It knows nothing
//! about workspaces or agents; the caller builds the rows.

use crate::render::boxed::{put_within, Boxed};
use crate::render::theme;
use domux_core::text::{
    display_width, sanitize_for_display, truncate_with_ellipsis, wrap_to_width,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

/// One row: the lines it draws, and what it acts on when it is not a header or a blank.
#[derive(Debug, Clone)]
pub struct ListRow {
    pub lines: Vec<Line<'static>>,
    /// The object the cursor acts on: a workspace id in M2, an agent id in M3. `None` means
    /// the cursor never rests here (interface spec 12.14).
    pub key: Option<String>,
    /// What `/` matches against, lower-cased by the caller's builder.
    pub filter_text: String,
}

impl ListRow {
    pub fn header(lines: Vec<Line<'static>>) -> ListRow {
        ListRow {
            lines,
            key: None,
            filter_text: String::new(),
        }
    }

    pub fn blank() -> ListRow {
        ListRow {
            lines: vec![Line::from("")],
            key: None,
            filter_text: String::new(),
        }
    }

    pub fn selectable(
        key: impl Into<String>,
        filter_text: impl Into<String>,
        lines: Vec<Line<'static>>,
    ) -> ListRow {
        ListRow {
            lines,
            key: Some(key.into()),
            filter_text: filter_text.into().to_lowercase(),
        }
    }

    /// The lines this row occupies. Saturating rather than truncating: a row with more lines
    /// than `u16` can hold would otherwise measure as a handful and scroll to the wrong line.
    pub fn height(&self) -> u16 {
        u16::try_from(self.lines.len()).unwrap_or(u16::MAX)
    }

    /// A row the cursor skips and that carries nothing to read: a separator, not a header.
    /// The filter drops these and puts its own blanks between the groups it keeps, so a
    /// blank left over from an unfiltered list never leads a group.
    pub fn is_blank(&self) -> bool {
        self.key.is_none()
            && self
                .lines
                .iter()
                .all(|line| line.spans.iter().all(|s| s.content.trim().is_empty()))
    }
}

/// A cell of empty space between the border and a row's text, on both sides.
///
/// Interface spec 5.2 says rows are flush with the box's left padding; before this there was
/// no padding to be flush with and the text touched the border. One cell puts a row's first
/// character directly under the first character of the box's title, which `Boxed` draws at
/// `area.x + 2`.
pub const PAD: u16 = 1;

/// The cells a row's text has inside a box `width` cells wide: the two borders and the two
/// pads taken off. Every surface that builds rows asks this, so the width a row truncates to
/// and the width it is drawn in are one number.
pub fn content_width(width: u16) -> u16 {
    width.saturating_sub(2 + 2 * PAD)
}

/// Whether a blank row goes between two rows of one group.
///
/// Either side saying more than its name is enough. A run of one-line rows stays tight and
/// reads as one block, and the moment a row has a second line the join on both sides of it is
/// marked: without the blank above, a one-line row sitting on top of a two-line one reads as
/// that row's first line, which is a workspace the reader can lose entirely.
///
/// The row builder writes the list to this rule and `filter_rows` rebuilds it to the same
/// one, so `/` changes what the list holds and never its shape.
pub fn needs_gap_between(above: &ListRow, below: &ListRow) -> bool {
    above.height() > 1 || below.height() > 1
}

pub struct ListBox<'a> {
    pub title: &'a str,
    pub rows: &'a [ListRow],
    /// The row index the fill sits on: the cursor when `focused`, the current row otherwise
    /// (domain model, section 3.3).
    pub filled: Option<usize>,
    pub focused: bool,
    /// The first visible line, remembered by the client so the view moves as little as it
    /// can. `render` corrects it and returns what it used.
    pub scroll: u16,
    /// What to draw when there are no rows: name the state and the next action
    /// (principle 9). Wrapped to the box's inner width, over as many rows as it has.
    pub empty_text: &'a str,
}

impl ListBox<'_> {
    /// Draws the box into `area` and answers with the scroll it used.
    ///
    /// It does not clear `area` first. The border is painted, and each row's own text, and
    /// the cells a row's text does not reach are left as they were found. **The caller owns
    /// whatever was under the box**: `overlay::frame` clears its rectangle before drawing
    /// one, and `sidebar::draw` clears `projects_area` for the same reason. Both happen to
    /// do it, which is not the same as it being written down, so it is written down here.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> u16 {
        let inner = Boxed {
            title: self.title,
            flag: None,
            focused: self.focused,
        }
        .render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return self.scroll;
        }
        let right = inner.x + inner.width - 1;
        // Where the text goes and how much of it fits. The fill still spans `inner`, so the
        // pad is inside the band rather than beside it.
        let text_x = inner.x + PAD;
        let text_width = inner.width.saturating_sub(2 * PAD);
        if self.rows.is_empty() {
            // Wrapped over the box's rows rather than cut at the first. The sentence names
            // the state and then the next action (principle 9), and the action is its second
            // half, so a box too narrow for one line would drop exactly the half the reader
            // is here for: the sidebar's Agents box has 34 columns for text inside its border
            // and its padding, and its text is 47 cells. Text that fits one line still takes
            // one, so nothing that fitted before has moved.
            //
            // A box with fewer rows than the text needs fills its last row from everything
            // that is left rather than from the next wrapped line, and ends in the mark that
            // says it was cut. Wrapping alone would show less than not wrapping at all: in a
            // box one row tall the first wrapped line is one word, where cutting the sentence
            // fills the row. So the rule is "wrap while there is room, then show as much as
            // fits", which is what the reader wants in both cases.
            //
            // The pad is the rows' pad: `draw_line` starts every row at `text_x` and stops at
            // `text_width`, so a box with no rows puts its one sentence where the rows would
            // have been rather than one column further left.
            if text_width == 0 {
                return 0;
            }
            let text = sanitize_for_display(self.empty_text);
            let lines = wrap_to_width(&text, text_width as usize);
            let room = inner.height as usize;
            for (n, line) in lines.iter().take(room).enumerate() {
                let cut = n + 1 == room && lines.len() > room;
                let text = match cut {
                    // `wrap_to_width` splits on whitespace, so joining the rest with one space
                    // is the text it was given, less the runs of spaces it already collapsed.
                    true => truncate_with_ellipsis(&lines[n..].join(" "), text_width as usize),
                    false => line.clone(),
                };
                put_within(
                    buf,
                    text_x,
                    inner.y + n as u16,
                    text_x + text_width - 1,
                    &text,
                    Style::default().fg(theme::OVERLAY0),
                );
            }
            return 0;
        }
        let scroll = scroll_to_show(self.rows, self.filled, inner.height, self.scroll);
        let bottom = scroll.saturating_add(inner.height);
        let mut next = 0u16;
        for (i, row) in self.rows.iter().enumerate() {
            for (n, line) in row.lines.iter().enumerate() {
                let y = next;
                next = next.saturating_add(1);
                if y < scroll || y >= bottom {
                    continue;
                }
                let at = inner.y + (y - scroll);
                // The fill is on line 1 of the row only (interface spec 5.3), and it covers
                // the whole inner width, not just the text: the row is the focus target, so
                // it reads as one band.
                let fill = self.filled == Some(i) && n == 0;
                if fill {
                    for x in inner.x..=right {
                        buf[(x, at)].set_style(Style::default().bg(theme::SURFACE0));
                    }
                }
                draw_line(line, text_x, at, text_width, fill, buf);
            }
        }
        scroll
    }
}

/// The index of the row drawn at screen row `y`, or `None` when no row is drawn there.
///
/// The same walk `render` makes, in the same order, over the same rows and scroll: a row is as
/// many lines tall as it has, and the lines outside the scrolled window are not drawn. A click
/// then lands on the row the reader sees, whatever the rows above it are.
pub fn row_at(rows: &[ListRow], scroll: u16, inner: Rect, y: u16) -> Option<usize> {
    if inner.height == 0 || y < inner.y || y >= inner.bottom() {
        return None;
    }
    let wanted = (y - inner.y).checked_add(scroll)?;
    let mut next = 0u16;
    for (i, row) in rows.iter().enumerate() {
        for _ in 0..row.height() {
            if next == wanted {
                return Some(i);
            }
            next = next.saturating_add(1);
        }
    }
    None
}

/// Draws one line's spans, cut to `width` by grapheme with a trailing ellipsis. The filled
/// line takes the fill as its background and brightens: bold text stays bold, and dim text
/// loses its dimming (interface spec 5.3).
fn draw_line(line: &Line<'static>, x: u16, y: u16, width: u16, fill: bool, buf: &mut Buffer) {
    let end = x + width;
    let mut cx = x;
    for span in &line.spans {
        if cx >= end {
            break;
        }
        let room = (end - cx) as usize;
        // Sanitize before measuring, as `Boxed` does with its title. A row's text carries
        // names that came from a directory, a branch or an agent, so a budget measured on
        // cells that will never be drawn would cut in the wrong place.
        let content = sanitize_for_display(&span.content);
        let cut = display_width(&content) > room;
        let text = if cut {
            truncate_with_ellipsis(&content, room)
        } else {
            content
        };
        let mut style = span.style;
        if fill {
            style = style.bg(theme::SURFACE0).remove_modifier(Modifier::DIM);
        }
        // `put_within` and not `put`: the budget above keeps text inside the border, and the
        // box's own right edge keeps a wrong budget from writing over it.
        cx = put_within(buf, cx, y, end - 1, &text, style);
        if cut {
            // The ellipsis ends the line. A wide grapheme dropped whole leaves a spare cell,
            // and a later span drawn into it reads as text that survived the cut when it fits,
            // and as a second ellipsis when it does not.
            break;
        }
    }
}

/// Keeps the rows whose `filter_text` contains `filter`, without case, and drops a header
/// whose rows all went with it. M3's Agents box filters the same way.
///
/// The blanks are rebuilt rather than kept, because the blank above a match is usually the
/// separator that led the group the filter just emptied. They are rebuilt to the grammar the
/// row builder uses, which the filter does not change: one blank before a header, and under
/// it whatever `needs_gap_between` asks for. Keeping a blank the builder would not have
/// written would let `/` change the shape of the list and not only its contents.
pub fn filter_rows(rows: &[ListRow], filter: &str) -> Vec<ListRow> {
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        return rows.to_vec();
    }
    let mut out: Vec<ListRow> = Vec::new();
    let mut header: Option<ListRow> = None;
    for row in rows {
        if row.key.is_none() {
            if !row.is_blank() {
                header = Some(row.clone());
            }
            continue;
        }
        if !row.filter_text.contains(&filter) {
            continue;
        }
        // A header opens a group and takes the blank before it; the first row under it sits
        // straight beneath. Inside a group the builder's own rule decides.
        if let Some(h) = header.take() {
            if !out.is_empty() {
                out.push(ListRow::blank());
            }
            out.push(h);
        } else if out
            .last()
            .is_some_and(|above| needs_gap_between(above, row))
        {
            out.push(ListRow::blank());
        }
        out.push(row.clone());
    }
    out
}

/// The first visible line so that the whole filled row is in view, moving as little as
/// possible from `scroll`. No sticky headers: a header scrolls away with its rows
/// (interface spec 12.2).
pub fn scroll_to_show(rows: &[ListRow], filled: Option<usize>, height: u16, scroll: u16) -> u16 {
    let total = rows
        .iter()
        .fold(0u16, |sum, r| sum.saturating_add(r.height()));
    let max = total.saturating_sub(height);
    let Some(filled) = filled else {
        return scroll.min(max);
    };
    let start = rows
        .iter()
        .take(filled)
        .fold(0u16, |sum, r| sum.saturating_add(r.height()));
    let end = start.saturating_add(rows.get(filled).map(|r| r.height()).unwrap_or(1));
    let mut scroll = scroll.min(max);
    if start < scroll || end.saturating_sub(start) > height {
        // Above the view, or taller than the view. A row that cannot fit shows its first
        // line, which is the line the fill sits on: scrolling to its last line instead would
        // put the row in view and the fill out of it.
        scroll = start;
    } else if end > scroll.saturating_add(height) {
        scroll = end - height;
    }
    scroll.min(max)
}
