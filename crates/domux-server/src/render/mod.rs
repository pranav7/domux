//! Composes one client's frame: the top bar, then the tab's pane boxes.

pub mod boxed;
pub mod pane_box;
pub mod tab_row;
pub mod theme;
pub mod top_bar;

use ratatui::layout::Rect;

pub fn to_rect(r: domux_core::model::Rect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}

use crate::pane::PaneRuntime;
use crate::render::boxed::Boxed;
use crate::render::pane_box::{cursor_position, render_grid};
use chrono::{DateTime, Local};
use domux_core::config::ConfigError;
use domux_core::ids::PaneId;
use domux_core::ids::TabId;
use domux_core::keymap::Keymap;
use domux_core::model::layout::solve;
use domux_core::model::{ClientView, Focus, Model};
use domux_core::proto::CursorState;
use domux_term::{Emulator, Size};
use ratatui::buffer::Buffer;
use std::collections::HashMap;

/// Under this the screen shows its size and the size it needs (principle 6).
pub const MIN_COLS: u16 = 40;
pub const MIN_ROWS: u16 = 10;

pub struct RenderInput<'a> {
    pub model: &'a Model,
    pub panes: &'a HashMap<PaneId, PaneRuntime>,
    pub view: &'a ClientView,
    pub keymap: &'a Keymap,
    pub now: DateTime<Local>,
    pub config_error: Option<&'a ConfigError>,
    pub hint: Option<&'a str>,
}

/// The workpanel: everything under the top bar.
pub fn workpanel_area(size: Size) -> domux_core::model::Rect {
    domux_core::model::Rect {
        x: 0,
        y: 1,
        width: size.cols,
        height: size.rows.saturating_sub(1),
    }
}

/// One client's whole screen: the top bar over the tab's pane boxes.
pub fn compose(input: &RenderInput) -> (Buffer, Option<CursorState>) {
    let size = input.view.size;
    let mut buf = Buffer::empty(Rect::new(0, 0, size.cols, size.rows));
    if size.cols < MIN_COLS || size.rows < MIN_ROWS {
        let msg = format!(
            "Screen is {}x{}. domux needs at least {}x{}.",
            size.cols, size.rows, MIN_COLS, MIN_ROWS
        );
        // Wrapped, not clipped. The sentence is wider than any screen that reaches this
        // branch - it is 43 cells and the branch is only taken below 40 columns - so a
        // single row would show two thirds of it and drop the size the screen has to reach,
        // which is the one fact the reader is here for (principle 9).
        let style = ratatui::style::Style::default().fg(theme::TEXT);
        for (y, line) in domux_core::text::wrap_to_width(&msg, size.cols as usize)
            .iter()
            .zip(0..size.rows)
            .map(|(line, y)| (y, line))
        {
            boxed::put(&mut buf, 0, y, line, style);
        }
        return (buf, None);
    }
    top_bar::draw(input, &mut buf);
    let cursor = draw_panes(input, &mut buf);
    (buf, cursor)
}

/// The size of the smallest client on `tab`; a tab nobody views keeps `fallback`.
///
/// Every client on a tab draws the same boxes, sized for the smallest of them, and the
/// larger ones leave the rest of the screen blank (tmux's rule). `Core::sync_pane_sizes`
/// sizes the PTYs from the same rectangle, so what a pane's program believes about its size
/// is what every client actually draws.
pub fn smallest_size(model: &Model, tab: &TabId, fallback: Size) -> Size {
    let mut size: Option<Size> = None;
    for c in model.clients.iter().filter(|c| &c.tab == tab) {
        let s = size.get_or_insert(c.size);
        s.cols = s.cols.min(c.size.cols);
        s.rows = s.rows.min(c.size.rows);
    }
    size.unwrap_or(fallback)
}

/// The size this client's pane boxes are laid out in: the smallest client's, and never
/// larger than the client's own screen.
///
/// The second clamp is load-bearing. `Boxed::render` and `render_grid` index the buffer
/// without checking their area against it, so an area past its edge panics rather than
/// clips, and this is the first place an area is computed from something other than the
/// buffer's own size: a client reports its size in its hello and its resizes, and nothing
/// clamps that. When the rendering view is one of `model.clients` the smallest is already
/// no larger, but `compose` is public and a caller can pass a view the model does not hold,
/// in which case a larger client on the tab would otherwise size this buffer's boxes.
fn drawn_size(input: &RenderInput, tab: &TabId) -> Size {
    let smallest = smallest_size(input.model, tab, input.view.size);
    Size {
        cols: smallest.cols.min(input.view.size.cols),
        rows: smallest.rows.min(input.view.size.rows),
    }
}

pub(crate) fn draw_panes(input: &RenderInput, buf: &mut Buffer) -> Option<CursorState> {
    let tab = input.model.tab(&input.view.tab)?;
    let area = workpanel_area(drawn_size(input, &tab.id));
    let pane_focus = matches!(input.view.focus, Focus::Pane(_));
    let mut cursor = None;
    for (pane_id, rect) in solve(&tab.layout, area, tab.zoomed.as_ref()) {
        let Some(pane) = input.model.pane(&pane_id) else {
            continue;
        };
        let runtime = input.panes.get(&pane_id);
        let focused = pane_focus && tab.focused == pane_id;
        let zoomed = tab.zoomed.as_ref() == Some(&pane_id);
        let copy = runtime.and_then(|r| r.copy.as_ref());
        let exited = runtime.and_then(|r| r.exited);
        let flag_text = flag(
            zoomed,
            copy.map(|c| c.offset),
            exited,
            runtime.map(|r| r.emulator.scrollback_len()).unwrap_or(0),
        );
        let title = pane.command.clone().unwrap_or_default();
        let inner = Boxed {
            title: &title,
            flag: flag_text.as_deref(),
            focused,
        }
        .render(to_rect(rect), buf);
        if let Some(rt) = runtime {
            let selection = copy.and_then(|c| c.selection());
            render_grid(&rt.grid, selection.as_ref(), inner, buf);
            if focused {
                let c = match copy {
                    Some(c) => domux_term::Cursor {
                        row: c.cursor.0,
                        col: c.cursor.1,
                        visible: true,
                        shape: domux_term::CursorShape::Block,
                        blink: false,
                    },
                    None => rt.emulator.cursor(),
                };
                cursor = cursor_position(inner, &c).map(|p| CursorState {
                    x: p.x,
                    y: p.y,
                    shape: c.shape,
                    blink: c.blink,
                });
            }
        }
    }
    cursor
}

/// The right-end flag: `copy`, `copy 120/3400`, `zoomed`, `exited 0`, joined with ` · `.
fn flag(
    zoomed: bool,
    copy_offset: Option<usize>,
    exited: Option<Option<i32>>,
    scrollback: usize,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(offset) = copy_offset {
        parts.push(if offset == 0 {
            "copy".to_string()
        } else {
            format!("copy {offset}/{scrollback}")
        });
    }
    if let Some(status) = exited {
        parts.push(match status {
            Some(s) => format!("exited {s}"),
            None => "exited".to_string(),
        });
    }
    if zoomed {
        parts.push("zoomed".to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workpanel_starts_under_the_top_bar_and_keeps_the_full_width() {
        let area = workpanel_area(Size { cols: 80, rows: 24 });
        assert_eq!((area.x, area.y), (0, 1));
        assert_eq!((area.width, area.height), (80, 23));
    }

    /// A screen one row tall has no room under the top bar. `saturating_sub` gives an empty
    /// workpanel rather than wrapping to 65535 rows.
    #[test]
    fn a_screen_with_no_room_under_the_top_bar_gets_an_empty_workpanel() {
        let area = workpanel_area(Size { cols: 80, rows: 0 });
        assert_eq!(area.height, 0);
    }

    #[test]
    fn the_flag_reads_copy_then_exited_then_zoomed() {
        assert_eq!(flag(false, None, None, 0), None);
        assert_eq!(flag(true, None, None, 0).as_deref(), Some("zoomed"));
        assert_eq!(flag(false, Some(0), None, 3400).as_deref(), Some("copy"));
        assert_eq!(
            flag(false, Some(120), None, 3400).as_deref(),
            Some("copy 120/3400")
        );
        assert_eq!(
            flag(false, None, Some(Some(0)), 0).as_deref(),
            Some("exited 0")
        );
        assert_eq!(
            flag(false, None, Some(None), 0).as_deref(),
            Some("exited"),
            "a status that did not arrive is absent, never a guessed 0"
        );
        assert_eq!(
            flag(true, Some(120), Some(Some(1)), 3400).as_deref(),
            Some("copy 120/3400 · exited 1 · zoomed")
        );
    }
}
