//! The sidebar: 38 columns down the left of the screen holding the Projects box, with the
//! hint row under it. M3 adds the Agents box beneath Projects (interface spec 12.25).
//!
//! This is the sidebar's geometry and its hint row. Nothing draws the Projects box into
//! `projects_area` yet: its rows are `render::projects_box`, which task 11 builds and the
//! switcher draws from the same builder, so the sidebar and the switcher cannot show one
//! project two ways. Task 13b adds the `draw` that puts a `ListBox` in this rectangle and
//! calls `hint_row` under it.

use crate::render::boxed::put;
use crate::render::{theme, RenderInput};
use domux_core::model::{Focus, RegionKind, SIDEBAR_WIDTH};
use domux_core::text::truncate_with_ellipsis;
use domux_term::Size;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

/// The bottom row of the sidebar's column: the keys, or the last result.
pub const HINT_ROW_HEIGHT: u16 = 1;

/// The sidebar's whole column, the full height of the screen.
pub fn sidebar_area(size: Size) -> Rect {
    Rect::new(0, 0, SIDEBAR_WIDTH.min(size.cols), size.rows)
}

/// The Projects box: everything above the hint row. In M3 this halves for the Agents box.
pub fn projects_area(size: Size) -> Rect {
    let area = sidebar_area(size);
    Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.saturating_sub(HINT_ROW_HEIGHT),
    )
}

/// `leader b hide · leader s search` normally; the keys of the cursor row while focus is in
/// the box; a pill while one is showing (interface spec 12.11 and 12.12).
pub fn hint_row(input: &RenderInput, area: Rect, buf: &mut Buffer) {
    let key = Style::default().fg(theme::BLUE);
    let word = Style::default().fg(theme::OVERLAY0);
    let sep = Style::default().fg(theme::SURFACE1);
    if area.y >= buf.area.bottom() {
        return;
    }
    let right = area.x.saturating_add(area.width).min(buf.area.right());
    for x in area.x..right {
        buf[(x, area.y)].reset();
    }
    if let Some(pill) = &input.view.pill {
        let style = Style::default()
            .fg(theme::BASE)
            .bg(if pill.ok { theme::GREEN } else { theme::RED })
            .add_modifier(Modifier::BOLD);
        put(
            buf,
            area.x + 1,
            area.y,
            &truncate_with_ellipsis(&pill.text, area.width.saturating_sub(2) as usize),
            style,
        );
        return;
    }
    // The keys as configured, never the default spelling (principle 3). A key the reader has
    // rebound to nothing drops out of the row rather than naming a key that does nothing.
    let focused = matches!(input.view.focus, Focus::Region(RegionKind::SidebarProjects));
    let pairs: Vec<(String, &str)> = if focused {
        [("list.activate", "open"), ("help", "more")]
            .iter()
            .filter_map(|(action, label)| input.keymap.list_key_for(action).map(|k| (k, *label)))
            .collect()
    } else {
        [("sidebar.toggle", "hide"), ("switcher.open", "search")]
            .iter()
            .filter_map(|(action, label)| input.keymap.hint_for(action).map(|k| (k, *label)))
            .collect()
    };
    let mut cx = area.x + 1;
    for (n, (k, label)) in pairs.iter().enumerate() {
        if n > 0 {
            cx = put(buf, cx, area.y, " · ", sep);
        }
        cx = put(buf, cx, area.y, k, key);
        cx = put(buf, cx, area.y, &format!(" {label}"), word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::ids::{ClientId, PaneId, TabId, WorkspaceId};
    use domux_core::keymap::Keymap;
    use domux_core::model::{ClientView, Model, Pill, TextInput};
    use domux_core::proto::Capabilities;
    use std::collections::HashMap;

    fn view(cols: u16, rows: u16) -> ClientView {
        ClientView {
            id: ClientId("c_0001".into()),
            size: Size { cols, rows },
            caps: Capabilities::default(),
            workspace: WorkspaceId("w_0001".into()),
            tab: TabId("t_0001".into()),
            focus: Focus::Pane(PaneId("p_0001".into())),
            sidebar_open: true,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            filtering: false,
            input: TextInput::new(""),
            overlay_under: None,
            pill: None,
        }
    }

    /// The hint row drawn into `area` of a buffer the caller has already prepared.
    fn hint_into(view: &ClientView, area: Rect, buf: &mut Buffer) {
        let model = Model::new(1);
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let input = RenderInput {
            model: &model,
            panes: &panes,
            view,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
        };
        hint_row(&input, area, buf);
    }

    /// The hint row drawn into `area` of a fresh `w` by `h` buffer.
    fn hint_buf(view: &ClientView, area: Rect, w: u16, h: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        hint_into(view, area, &mut buf);
        buf
    }

    fn line_of(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// The hint row alone, as text, over a screen 38 columns wide.
    fn row_of(view: &ClientView) -> String {
        line_of(&hint_buf(view, Rect::new(0, 0, 38, 1), 38, 1), 0)
    }

    #[test]
    fn the_hint_row_names_the_keys_that_hide_the_sidebar_and_open_the_switcher() {
        assert_eq!(
            row_of(&view(120, 24)),
            " leader b hide · leader s search      "
        );
    }

    /// The keys of the row the cursor is on, not the sidebar's own keys, once focus is in
    /// the box (interface spec 12.11).
    #[test]
    fn the_hint_row_names_the_rows_keys_while_focus_is_in_the_box() {
        let mut v = view(120, 24);
        v.focus = Focus::Region(RegionKind::SidebarProjects);
        assert_eq!(row_of(&v), " ⏎ open · ? more                      ");
    }

    /// A result displaces the keys until it clears (interface spec 12.12).
    #[test]
    fn a_pill_takes_the_hint_row_while_it_is_showing() {
        let mut v = view(120, 24);
        v.pill = Some(Pill {
            text: "workspace 2 cleared".into(),
            ok: true,
            at: "2026-09-04T14:32:00+01:00".into(),
        });
        assert_eq!(row_of(&v), " workspace 2 cleared                  ");
    }

    /// 38 columns of sidebar the whole height of the screen, and the bottom row of it is the
    /// hint row, so the box above it is one row shorter.
    #[test]
    fn the_sidebar_is_38_columns_and_the_box_stops_one_row_above_the_bottom() {
        let size = Size {
            cols: 120,
            rows: 24,
        };
        let bar = sidebar_area(size);
        assert_eq!((bar.x, bar.y, bar.width, bar.height), (0, 0, 38, 24));
        let box_area = projects_area(size);
        assert_eq!(
            (box_area.x, box_area.y, box_area.width, box_area.height),
            (0, 0, 38, 23)
        );
    }

    /// Nothing this narrow draws a sidebar, but the rectangle must stay inside the screen
    /// rather than reach past it: `Boxed::render` indexes the buffer without checking.
    #[test]
    fn a_screen_narrower_than_the_sidebar_gets_a_sidebar_no_wider_than_itself() {
        let bar = sidebar_area(Size { cols: 20, rows: 24 });
        assert_eq!(bar.width, 20);
    }

    /// The row draws into the area it is handed, not at a fixed place.
    ///
    /// Every other test here passes an area at the buffer's own origin, where a function
    /// that ignored the area entirely would look identical. Task 13b calls this with an
    /// area at column 0 too, so nothing in the running program would separate them either.
    #[test]
    fn the_hint_row_draws_where_its_area_says_and_nowhere_else() {
        let buf = hint_buf(&view(120, 24), Rect::new(10, 2, 38, 1), 60, 3);
        assert_eq!(
            line_of(&buf, 2),
            format!(
                "{}leader b hide · leader s search{}",
                " ".repeat(11),
                " ".repeat(18)
            )
        );
        assert_eq!(
            line_of(&buf, 0),
            " ".repeat(60),
            "the rows the area does not name are untouched"
        );
        assert_eq!(line_of(&buf, 1), " ".repeat(60));
    }

    /// A pill longer than the row is cut to it, with a mark to say it was cut. Every other
    /// pill here is short enough that the cut never fires, so a wrong budget would not show.
    #[test]
    fn a_pill_wider_than_the_hint_row_is_cut_to_it() {
        let mut v = view(120, 24);
        v.pill = Some(Pill {
            text: "w".repeat(80),
            ok: true,
            at: String::new(),
        });
        assert_eq!(row_of(&v), format!(" {}… ", "w".repeat(35)));
    }

    /// Green when it worked, red when it was refused (interface spec 7.3).
    ///
    /// The colours are interface spec 9.1's literals rather than `theme::GREEN` and
    /// `theme::RED`, so a token that moved would fail here instead of moving the assertion
    /// with it.
    #[test]
    fn a_refused_pill_is_red_and_a_good_one_is_green() {
        let mut v = view(120, 24);
        v.pill = Some(Pill {
            text: "main can't be deleted".into(),
            ok: false,
            at: String::new(),
        });
        let area = Rect::new(0, 0, 38, 1);
        assert_eq!(
            hint_buf(&v, area, 38, 1)[(1, 0)].bg,
            ratatui::style::Color::Rgb(0xf3, 0x8b, 0xa8)
        );
        v.pill.as_mut().unwrap().ok = true;
        assert_eq!(
            hint_buf(&v, area, 38, 1)[(1, 0)].bg,
            ratatui::style::Color::Rgb(0xa6, 0xe3, 0xa1)
        );
    }

    /// The row belongs to the hint row: whatever was in it before is gone.
    ///
    /// Every other test here hands it a fresh buffer, where a function that never cleared
    /// would look identical. Task 13b draws the Projects box into the rows above this one
    /// and M3 adds the Agents box, so what this row does with what it finds is a promise
    /// worth stating rather than a detail.
    #[test]
    fn the_hint_row_clears_what_was_in_its_row_before() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 38, 1));
        for x in 0..38 {
            buf[(x, 0)].set_symbol("X");
        }
        hint_into(&view(120, 24), Rect::new(0, 0, 38, 1), &mut buf);
        assert_eq!(line_of(&buf, 0), " leader b hide · leader s search      ");
    }
}
