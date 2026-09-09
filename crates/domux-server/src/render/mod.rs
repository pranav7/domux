//! Composes one client's frame: the top bar over the tab's pane boxes, or, when the sidebar
//! is open, the sidebar beside them with the tab row on top of them (interface spec 4.2).

pub mod agents_box;
pub mod agents_overlay;
pub mod boxed;
pub mod confirm;
pub mod list_box;
pub mod name_box;
pub mod overlay;
pub mod pane_box;
pub mod projects_box;
pub mod sidebar;
pub mod switcher;
pub mod tab_row;
pub mod theme;
pub mod top_bar;

use ratatui::layout::Rect;

pub fn to_rect(r: domux_core::model::Rect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}

use crate::client::Hint;
use crate::pane::PaneRuntime;
use crate::render::boxed::Boxed;
use crate::render::pane_box::{cursor_position, render_grid};
use chrono::{DateTime, Local};
use domux_core::config::ConfigError;
use domux_core::ids::PaneId;
use domux_core::ids::TabId;
use domux_core::keymap::Keymap;
use domux_core::model::layout::solve;
use domux_core::model::{ClientView, Focus, Model, Overlay, SIDEBAR_WIDTH};
use domux_core::proto::CursorState;
use domux_term::{Emulator, Size};
use ratatui::buffer::Buffer;
use std::collections::HashMap;

/// Under this the screen shows its size and the size it needs (principle 6).
pub const MIN_COLS: u16 = 40;
pub const MIN_ROWS: u16 = 10;

pub struct RenderInput<'a> {
    pub model: &'a Model,
    /// What domux observed about each workspace. The Projects box reads a branch and a pull
    /// request from here; a fact that did not arrive draws as absent, never as a guess
    /// (principle 4).
    pub facts: &'a crate::facts::FactRegistry,
    pub panes: &'a HashMap<PaneId, PaneRuntime>,
    /// Every agent this frame draws, in sort order, with its place, its recap and this
    /// frame's glyph and word already resolved. The Agents box reads nothing else, so the
    /// sidebar and the agents overlay cannot disagree about what an agent is doing.
    pub agents: &'a agents_box::AgentsView,
    pub view: &'a ClientView,
    pub keymap: &'a Keymap,
    pub now: DateTime<Local>,
    pub config_error: Option<&'a ConfigError>,
    /// One line for the clock's place. Its kind is what decides where it sits in the right
    /// end's priority order, so the whole hint is passed rather than its text: see
    /// `top_bar::right_end`.
    pub hint: Option<&'a Hint>,
    /// What the start-up prune took away, for the switcher's footer and the sidebar's hint
    /// row. Empty in every frame after the reader's first key in a box. See `note_line`.
    pub notes: &'a [String],
}

/// The one line a list of notes prints as, or `None` when there is nothing to say.
///
/// One function for both rows: the footer and the hint row draw it in their own widths and
/// their own styles, but a note cannot read one way in the switcher and another in the
/// sidebar. Two prunes join with the separator the hint rows already use, so a start that
/// took two records away says both rather than the first and a count.
pub fn note_line(notes: &[String]) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    Some(notes.join(" · "))
}

impl<'a> RenderInput<'a> {
    /// The pane the keys go to, or `None` when they go to a prompt, an overlay or nothing.
    ///
    /// `Focus::Pane`'s own payload is deliberately not read. `Core::focused_pane` is what
    /// decides where a key actually goes, and it discards the payload and answers `tab.focused`,
    /// so `tab.focused` is the pane with the keys and the focus variant only says whether a pane
    /// has them at all. A renderer reading the payload instead could offer one pane's mode keys
    /// while another pane was taking them. If M2 gives clients independent focus, the payload
    /// becomes the answer in `Core::focused_pane` first and this follows it - one question, one
    /// implementation, in that order.
    pub fn focused_pane(&self) -> Option<&'a PaneId> {
        match self.view.focus {
            Focus::Pane(_) => Some(&self.model.tab(&self.view.tab)?.focused),
            Focus::Region(_) => None,
        }
    }
}

/// The panes' rectangle on a screen of `size`, with or without the sidebar beside it.
///
/// One column of gap between the sidebar and the workpanel (interface spec 12.18), so the
/// pane boxes never share a column with the sidebar's border.
pub fn workpanel_of(size: Size, sidebar: bool) -> domux_core::model::Rect {
    let x = if sidebar { SIDEBAR_WIDTH + 1 } else { 0 };
    domux_core::model::Rect {
        x,
        y: 1,
        width: size.cols.saturating_sub(x),
        height: size.rows.saturating_sub(1),
    }
}

/// One client's workpanel: under the top bar when the sidebar is hidden, right of the
/// sidebar and under the tab row when it is shown.
pub fn workpanel_area(view: &ClientView) -> domux_core::model::Rect {
    workpanel_of(view.size, view.sidebar_visible())
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
    // With the sidebar shown there is no full-width top bar: the tab row sits on the panes
    // with the right end's pieces at its end (interface spec 4.2).
    if input.view.sidebar_visible() {
        sidebar::draw(input, &mut buf);
        tab_row::draw_workpanel_row(input, &mut buf);
    } else {
        top_bar::draw(input, &mut buf);
    }
    let cursor = draw_panes(input, &mut buf);
    overlay::draw(input, &mut buf);
    // An overlay other than the prompt covers the pane the cursor is in, so the outer terminal
    // hides the cursor rather than blinking it under the box: the overlay is the one visible
    // focus target (principle 2). The prompt draws its own caret in the tab cell.
    let cursor = match &input.view.overlay {
        None | Some(Overlay::Prompt(_)) => cursor,
        Some(_) => None,
    };
    (buf, cursor)
}

/// Whether this client draws pane boxes at all. A screen under the minimum shows only the
/// size notice, so it has no claim on a pane box or its PTY.
fn draws_panes(view: &ClientView) -> bool {
    view.size.cols >= MIN_COLS && view.size.rows >= MIN_ROWS
}

/// The smallest workpanel among the clients that draw panes on `tab`, or `None` when no
/// client draws it.
///
/// Only the width and the height are agreed on. Where the rectangle sits is each client's
/// own business: two clients that disagree about the sidebar draw the same boxes at
/// different columns, and the larger screen leaves the rest blank.
fn smallest_among(model: &Model, tab: &TabId) -> Option<domux_core::model::Rect> {
    let mut area: Option<domux_core::model::Rect> = None;
    for view in model
        .clients
        .iter()
        .filter(|view| &view.tab == tab && draws_panes(view))
    {
        let theirs = workpanel_area(view);
        match &mut area {
            None => area = Some(theirs),
            Some(area) => {
                area.width = area.width.min(theirs.width);
                area.height = area.height.min(theirs.height);
            }
        }
    }
    area
}

/// The workpanel every client on `tab` agrees on when no particular client is asking: the
/// size a PTY on that tab takes.
///
/// When no client draws the tab, `fallback` shapes it, raised to at least the minimum: a
/// caller without a current pane size gets a sane size rather than the tiny notice screen.
/// The fallback is not an upper bound on the clients - a client wider than it still draws
/// its own width - so it never shrinks a pane that something is actually drawing.
pub fn tab_workpanel(model: &Model, tab: &TabId, fallback: Size) -> domux_core::model::Rect {
    smallest_among(model, tab).unwrap_or_else(|| {
        workpanel_of(
            Size {
                cols: fallback.cols.max(MIN_COLS),
                rows: fallback.rows.max(MIN_ROWS),
            },
            false,
        )
    })
}

/// The same rectangle at `view`'s own position, and never larger than `view`'s own screen.
///
/// The second clamp is load-bearing. `Boxed::render` and `render_grid` index the buffer
/// without checking their area against it, so an area past its edge panics rather than
/// clips, and this is the first place an area is computed from something other than the
/// buffer's own size: a client reports its size in its hello and its resizes, and nothing
/// clamps that. When the rendering view is one of `model.clients` the smallest is already
/// no larger, but `compose` is public and a caller can pass a view the model does not hold,
/// in which case a larger client on the tab would otherwise size this buffer's boxes.
pub fn smallest_workpanel(
    model: &Model,
    tab: &TabId,
    view: &ClientView,
) -> domux_core::model::Rect {
    let here = workpanel_area(view);
    let mut area = smallest_among(model, tab).unwrap_or(here);
    area.x = here.x;
    area.y = here.y;
    area.width = area.width.min(here.width);
    area.height = area.height.min(here.height);
    area
}

pub(crate) fn draw_panes(input: &RenderInput, buf: &mut Buffer) -> Option<CursorState> {
    let tab = input.model.tab(&input.view.tab)?;
    let area = smallest_workpanel(input.model, &tab.id, input.view);
    let focused_pane = input.focused_pane();
    let mut cursor = None;
    for (pane_id, rect) in solve(&tab.layout, area, tab.zoomed.as_ref()) {
        let Some(pane) = input.model.pane(&pane_id) else {
            continue;
        };
        let runtime = input.panes.get(&pane_id);
        let focused = focused_pane == Some(&pane_id);
        let zoomed = tab.zoomed.as_ref() == Some(&pane_id);
        let copy = runtime.and_then(|r| r.copy.as_ref());
        let exited = runtime.and_then(|r| r.exited);
        // The rows copy mode may walk, which is not the same question as how many rows the
        // emulator is holding: see `copy_mode::history`.
        let history = runtime.map(crate::copy_mode::history).unwrap_or(0);
        let flag_text = flag(zoomed, copy.map(|c| c.offset), exited, history);
        let title = pane.command.clone().unwrap_or_default();
        let inner = Boxed {
            title: &title,
            flag: flag_text.as_deref(),
            focused,
        }
        .render(to_rect(rect), buf);
        if let Some(rt) = runtime {
            let selection = copy.and_then(|c| c.selection_at(history));
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

    use domux_core::ids::ClientId;

    /// A client on `tab` with a screen of `cols` x `rows` and its remembered sidebar state.
    fn view(id: &str, tab: &str, cols: u16, rows: u16, sidebar_open: bool) -> ClientView {
        ClientView {
            id: ClientId(id.to_string()),
            size: Size { cols, rows },
            tab: TabId(tab.to_string()),
            sidebar_open,
            ..crate::testing::client_view()
        }
    }

    fn model_with(views: Vec<ClientView>) -> Model {
        let mut m = Model::new(1);
        m.clients = views;
        m
    }

    #[test]
    fn the_workpanel_starts_under_the_top_bar_and_keeps_the_full_width() {
        let area = workpanel_of(Size { cols: 80, rows: 24 }, false);
        assert_eq!((area.x, area.y), (0, 1));
        assert_eq!((area.width, area.height), (80, 23));
    }

    /// 38 columns of sidebar, then one column of gap, so the panes start at 39 and a
    /// 120-column screen leaves them 81 (interface spec 12.18).
    #[test]
    fn the_sidebar_and_its_gap_push_the_workpanel_to_column_39() {
        let area = workpanel_of(
            Size {
                cols: 120,
                rows: 24,
            },
            true,
        );
        assert_eq!((area.x, area.y), (39, 1));
        assert_eq!((area.width, area.height), (81, 23));
    }

    /// A screen one row tall has no room under the top bar. `saturating_sub` gives an empty
    /// workpanel rather than wrapping to 65535 rows.
    #[test]
    fn a_screen_with_no_room_under_the_top_bar_gets_an_empty_workpanel() {
        let area = workpanel_of(Size { cols: 80, rows: 0 }, false);
        assert_eq!(area.height, 0);
    }

    /// Nothing narrower than the sidebar ever draws it, but the arithmetic must not wrap if
    /// a caller asks anyway.
    #[test]
    fn a_screen_narrower_than_the_sidebar_gets_an_empty_workpanel() {
        let area = workpanel_of(Size { cols: 10, rows: 24 }, true);
        assert_eq!(area.width, 0);
    }

    /// The remembered state alone does not draw the sidebar: the screen has to be wide
    /// enough too, and at 119 columns it is not (interface spec 12.1).
    #[test]
    fn a_view_takes_the_sidebar_rectangle_only_while_the_sidebar_is_visible() {
        assert_eq!(
            workpanel_area(&view("c_0001", "t_0001", 120, 24, true)).x,
            39
        );
        assert_eq!(
            workpanel_area(&view("c_0001", "t_0001", 119, 24, true)).x,
            0
        );
        assert_eq!(
            workpanel_area(&view("c_0001", "t_0001", 119, 24, true)).width,
            119,
            "the auto-hidden sidebar gives its columns back to the panes"
        );
        assert_eq!(
            workpanel_area(&view("c_0001", "t_0001", 120, 24, false)).x,
            0
        );
    }

    #[test]
    fn a_tab_no_client_draws_takes_the_fallback_raised_to_the_minimum() {
        let m = model_with(Vec::new());
        let area = tab_workpanel(&m, &TabId("t_0001".into()), Size { cols: 80, rows: 24 });
        assert_eq!((area.x, area.width, area.height), (0, 80, 23));
        let area = tab_workpanel(&m, &TabId("t_0001".into()), Size { cols: 4, rows: 2 });
        assert_eq!(
            (area.width, area.height),
            (40, 9),
            "raised to the 40x10 minimum, not adopted as a 4x2 screen"
        );
    }

    /// The fallback shapes the rectangle only when nothing draws the tab. A client wider
    /// than it keeps its own width, or every pane on a tab nobody asked about first would
    /// be squeezed to 80 columns.
    #[test]
    fn the_fallback_never_shrinks_a_tab_a_client_does_draw() {
        let m = model_with(vec![view("c_0001", "t_0001", 200, 50, false)]);
        let area = tab_workpanel(&m, &TabId("t_0001".into()), Size { cols: 80, rows: 24 });
        assert_eq!((area.width, area.height), (200, 49));
    }

    #[test]
    fn the_smallest_client_on_the_tab_sizes_the_workpanel() {
        let m = model_with(vec![
            view("c_0001", "t_0001", 200, 50, false),
            view("c_0002", "t_0001", 100, 30, false),
        ]);
        let area = tab_workpanel(&m, &TabId("t_0001".into()), Size { cols: 80, rows: 24 });
        assert_eq!((area.width, area.height), (100, 29));
    }

    /// A screen under the minimum shows only the size notice, so it draws no pane box and
    /// has no claim on one.
    #[test]
    fn a_client_too_small_to_draw_panes_does_not_size_them() {
        let m = model_with(vec![
            view("c_0001", "t_0001", 120, 40, false),
            view("c_0002", "t_0001", 20, 5, false),
        ]);
        let area = tab_workpanel(&m, &TabId("t_0001".into()), Size { cols: 80, rows: 24 });
        assert_eq!((area.width, area.height), (120, 39));
    }

    #[test]
    fn a_client_on_another_tab_does_not_size_this_one() {
        let m = model_with(vec![
            view("c_0001", "t_0001", 200, 50, false),
            view("c_0002", "t_0002", 41, 11, false),
        ]);
        let area = tab_workpanel(&m, &TabId("t_0001".into()), Size { cols: 80, rows: 24 });
        assert_eq!((area.width, area.height), (200, 49));
    }

    /// Two clients that disagree about the sidebar draw boxes of one size, each at its own
    /// columns: the wider screen leaves the rest blank rather than moving its boxes.
    #[test]
    fn the_agreed_workpanel_sits_at_the_asking_clients_own_columns() {
        let asking = view("c_0001", "t_0001", 120, 24, true);
        let m = model_with(vec![
            asking.clone(),
            view("c_0002", "t_0001", 200, 50, false),
        ]);
        let area = smallest_workpanel(&m, &TabId("t_0001".into()), &asking);
        assert_eq!((area.x, area.y), (39, 1), "this client's own position");
        assert_eq!(
            (area.width, area.height),
            (81, 23),
            "the smaller client's size"
        );
        let other = view("c_0002", "t_0001", 200, 50, false);
        let area = smallest_workpanel(&m, &TabId("t_0001".into()), &other);
        assert_eq!((area.x, area.width), (0, 81));
    }

    /// `compose` is public and a caller can pass a view the model does not hold. The boxes
    /// are still clamped to that view's own screen, because `Boxed::render` indexes the
    /// buffer without checking and an area past its edge panics rather than clips.
    #[test]
    fn a_view_the_model_does_not_hold_still_clamps_to_its_own_screen() {
        let m = model_with(vec![view("c_0001", "t_0001", 200, 50, false)]);
        let stranger = view("c_0009", "t_0001", 60, 20, false);
        let area = smallest_workpanel(&m, &TabId("t_0001".into()), &stranger);
        assert_eq!((area.width, area.height), (60, 19));
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
