//! The agents overlay: one overlay holding the Agents box with its footer on the box's last
//! row, under `leader a` (domain model, section 3.6).
//!
//! It is the switcher's shape with a different box in it, and `render::switcher` is the
//! worked example this follows: the same `list_overlay_area` geometry, the same `clear` and
//! `dim`, the same `ListBox`, the same footer row inside it. Only the rows and the footer's
//! words differ, so an overlay cannot start reading one way and the switcher another
//! (principle 14).
//!
//! The rows are `agents_box::rows`, the same builder the sidebar's box uses, so one agent is
//! one agent on both surfaces. What this asks for is the wider form: the place carries its
//! tab and the recap is drawn, neither of which fits the sidebar's 38 columns. The empty
//! text is `agents_box::empty_text` for the same reason.

use crate::render::agents_box::{self, rows, RowForm, TITLE};
use crate::render::list_box::{
    box_lines, content_width, filter_rows, footer_area, ListBox, OVERLAY_PAD,
};
use crate::render::projects_box::filled_index;
use crate::render::{overlay, RenderInput};
use ratatui::buffer::Buffer;

/// `⏎ open · / filter · esc close` (interface spec 6.8). Each entry is an action, looked up
/// in `[keys.list]` when the row is drawn, so a rebinding shows here (principle 3).
///
/// One hint shorter than the switcher's, which also offers `? help`. The Agents box's rows
/// are three lines each where a workspace row is one, so the box is the shorter list of the
/// two and the row it stands on is the same width; the help key is the one the reader is
/// least likely to be reaching for here.
pub const FOOTER: [(&str, &str); 3] = [
    ("list.activate", "open"),
    ("list.filter", "filter"),
    ("focus.pane", "close"),
];

pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    let screen = buf.area;
    // Two passes, as the switcher makes them: the rows truncate to the box's inner width and
    // the row count then decides the box's height, so the width answers first because it does
    // not depend on the rows.
    let inner_width = content_width(overlay::list_overlay_width(screen), OVERLAY_PAD);
    let all = rows(input.agents, RowForm::Overlay, inner_width);
    let visible = filter_rows(&all, &input.view.filter);
    let lines = visible
        .iter()
        .fold(0u16, |sum, r| sum.saturating_add(r.height()));
    let area = overlay::list_overlay_area(screen, box_lines(lines, OVERLAY_PAD));
    // The cursor is the agent under it, not a row number, so a re-sort between two frames
    // keeps the fill on the agent it was on (principle 2).
    let cursor = input.view.agents_cursor.as_ref().map(|id| id.to_string());
    let filled = filled_index(&visible, cursor.as_deref());
    let empty = agents_box::empty_text(&input.view.filter, RowForm::Overlay);
    // `clear` and not `frame_at`: a `ListBox` draws its own border, so the agents overlay and
    // the sidebar share one drawing of the Agents box.
    overlay::clear(area, buf);
    ListBox {
        title: TITLE,
        rows: &visible,
        filled,
        focused: true,
        scroll: input.view.agents_scroll,
        empty_text: &empty,
        pad: OVERLAY_PAD,
    }
    .render(area, buf);
    // The footer's row is the box's last, inside the border, the same as the switcher's
    // (MUX-16).
    if let Some(footer) = footer_area(area, OVERLAY_PAD) {
        overlay::footer(input, &FOOTER, footer, buf);
    }
    // Last, so that what is dimmed is what the overlay did not draw. The corrected scroll
    // `render` returns is dropped on purpose: a renderer does not write to the model, and
    // `list.down` and `list.up` own `agents_scroll`.
    overlay::dim(buf, &[area]);
}
