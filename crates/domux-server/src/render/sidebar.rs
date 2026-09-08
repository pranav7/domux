//! The sidebar: 38 columns down the left of the screen holding the Projects box, with the
//! hint row under it. M3 adds the Agents box beneath Projects (interface spec 12.25).
//!
//! The rows come from `render::projects_box`, which the switcher draws from too, so the
//! sidebar and the switcher cannot show one project two ways.

use crate::render::boxed::put;
use crate::render::list_box::ListBox;
use crate::render::projects_box::{rows, Extras, PROJECTS_TITLE};
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

/// The Projects box in the sidebar's column, with the hint row under it.
pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    let size = input.view.size;
    let area = projects_area(size);
    // The box's own rectangle first, the way `overlay::frame` clears before drawing its box.
    // `ListBox` paints its border and the text of each row, and leaves the cells after a row's
    // text as it found them, so the region has to be cleared by whoever owns it. Today
    // `compose` hands over a fresh buffer and nothing would show, which is exactly why this is
    // written down rather than assumed: M3 puts the Agents box in this same column.
    let right = area.x.saturating_add(area.width).min(buf.area.right());
    let bottom = area.y.saturating_add(area.height).min(buf.area.bottom());
    for y in area.y..bottom {
        for x in area.x..right {
            buf[(x, y)].reset();
        }
    }
    let focused = matches!(input.view.focus, Focus::Region(RegionKind::SidebarProjects));
    // The fill marks the row the keys act on: the cursor while focus is in the box, and the
    // workspace this client is in otherwise (domain model, section 3.3). `rows` is given the
    // key and hands back where it put the fill, so the band and the brightening cannot land
    // on different rows.
    let key = match focused {
        true => input.view.projects_cursor.as_ref().map(|w| w.as_str()),
        false => None,
    }
    .unwrap_or(input.view.workspace.as_str());
    let built = rows(
        input.model,
        input.facts,
        &input.view.filter,
        Some(key),
        Extras::compact(area.width.saturating_sub(2)),
    );
    let empty = format!(
        "No projects yet. {} open <path>",
        domux_core::names::BIN_NAME
    );
    ListBox {
        title: PROJECTS_TITLE,
        rows: &built.rows,
        filled: built.filled,
        focused,
        scroll: input.view.projects_scroll,
        empty_text: &empty,
    }
    .render(area, buf);
    hint_row(
        input,
        Rect::new(0, size.rows.saturating_sub(1), area.width, 1),
        buf,
    );
}

/// `leader b hide · leader s search` normally; the keys of the cursor row while focus is in
/// the box; a pill while one is showing (interface spec 12.11 and 12.12).
pub fn hint_row(input: &RenderInput, area: Rect, buf: &mut Buffer) {
    let key = Style::default().fg(theme::BLUE);
    let word = Style::default().fg(theme::OVERLAY0);
    let sep = Style::default().fg(theme::SURFACE1);
    // Both bounds are guards with no test, deliberately: nothing in the running program
    // passes an area outside the buffer, and `buf[(x, y)]` panics rather than clips, so
    // being wrong costs a crash and the guard costs two comparisons. The same argument
    // `sidebar_area` carries for clamping its width to the screen.
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
    use std::path::PathBuf;

    fn view(cols: u16, rows: u16) -> ClientView {
        ClientView {
            id: ClientId("c_0001".into()),
            size: Size { cols, rows },
            caps: Capabilities::default(),
            workspace: WorkspaceId("w_0001".into()),
            tab: TabId("t_0001".into()),
            focus: Focus::Pane(PaneId("p_0001".into())),
            sidebar_open: true,
            sidebar_forced: false,
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

    /// The whole sidebar drawn into a buffer the caller has already prepared, over a git
    /// project with `main` and one named slot, so the cursor and the current workspace can
    /// be different rows. `tweak` gets the view and the two workspace ids.
    fn draw_two(
        buf: &mut Buffer,
        tweak: impl FnOnce(&mut ClientView, &domux_core::ids::WorkspaceId),
    ) {
        let mut model = Model::new(7);
        let (pid, ws, _) = model
            .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
            .unwrap();
        let (slot, _) = model
            .add_slot(
                &pid,
                1,
                PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1"),
            )
            .unwrap();
        let (tab, pane, _) = model
            .create_tab(&ws, PathBuf::from("/repo/audrey-app"))
            .unwrap();
        let mut v = view(120, 24);
        v.workspace = ws;
        v.tab = tab;
        v.focus = Focus::Pane(pane);
        tweak(&mut v, &slot);
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let facts = crate::facts::FactRegistry::new();
        let input = RenderInput {
            model: &model,
            panes: &panes,
            view: &v,
            keymap: &keymap,
            facts: &facts,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
        };
        draw(&input, buf);
    }

    /// The sidebar's rows as text, one string per line inside the box.
    fn box_rows(buf: &Buffer) -> Vec<String> {
        (1..22)
            .map(|y| {
                (1..37)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// The hint row drawn into `area` of a buffer the caller has already prepared.
    fn hint_into(view: &ClientView, area: Rect, buf: &mut Buffer) {
        let model = Model::new(1);
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let facts = crate::facts::FactRegistry::new();
        let input = RenderInput {
            model: &model,
            panes: &panes,
            view,
            keymap: &keymap,
            facts: &facts,
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

    /// The box covers what was under it, across its whole rectangle.
    ///
    /// Every other render test in this repository starts from a fresh `Buffer::empty`, where
    /// a box that painted only its border and its rows is indistinguishable from one that
    /// also cleared the cells between them. `compose` does hand it a fresh buffer today, so
    /// nothing would notice; M3 puts the Agents box in this same column and interface spec
    /// 12.18's one-cell gap is the only thing between them, so what this box does with what
    /// it finds is a promise worth holding it to now rather than after something relies on
    /// it.
    #[test]
    fn the_projects_box_covers_what_was_under_it() {
        // `@` because nothing the box draws can produce it. **A canary has to be a value the
        // system under test cannot produce**, or it cannot tell "left behind" from "drawn".
        // The first version of this test filled with `X`, and the project at `/x` drew its
        // header as `X`, so a correct box read as one that had left the fill behind. Do not
        // tidy this back to a letter.
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        for y in 0..24 {
            for x in 0..120 {
                buf[(x, y)].set_symbol("@");
            }
        }
        draw_two(&mut buf, |_, _| {});
        for y in 0..24 {
            let line: String = (0..38).map(|x| buf[(x, y)].symbol()).collect();
            assert!(
                !line.contains('@'),
                "row {y} of the sidebar's column still shows what was under it: {line:?}"
            );
        }
        // The column beside it is untouched: the box covers its own rectangle and no more.
        let line: String = (38..120).map(|x| buf[(x, 5)].symbol()).collect();
        assert_eq!(line, "@".repeat(82), "the box painted past its own width");
    }

    /// The text of the row carrying the fill, or `None` if no row does.
    fn filled_row(buf: &Buffer) -> Option<String> {
        (1..22u16)
            .find(|y| buf[(2u16, *y)].bg != ratatui::style::Color::Reset)
            .map(|y| {
                (1..37u16)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
    }

    /// The cursor moves the fill only while focus is in the box.
    ///
    /// Nothing sets `projects_cursor` in M2 - list navigation arrives later - so every test
    /// that goes through the harness leaves it `None`, where following the cursor and
    /// ignoring it look identical. Here it is set directly, to a workspace that is not the
    /// one this client is in, so the two answers are different rows.
    #[test]
    fn the_cursor_moves_the_fill_only_while_focus_is_in_the_box() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |v, slot| v.projects_cursor = Some(slot.clone()));
        assert_eq!(
            filled_row(&buf).as_deref(),
            Some("main"),
            "the keys are in a pane, so the fill stays on the workspace this client is in"
        );

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |v, slot| {
            v.projects_cursor = Some(slot.clone());
            v.focus = Focus::Region(RegionKind::SidebarProjects);
        });
        assert_eq!(
            filled_row(&buf).as_deref(),
            Some("workspace-1"),
            "with the keys in the box the fill follows the cursor"
        );
    }

    /// The filter drops the rows it does not match.
    ///
    /// `ClientView::filter` is only written by `list.filter`, which M2 has not built, so
    /// every harness test leaves it empty and a box that ignored it entirely would pass.
    #[test]
    fn the_filter_drops_the_rows_it_does_not_match() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |v, _| v.filter = "workspace-1".into());
        let rows = box_rows(&buf);
        let text = rows.join("|");
        assert!(
            !text.contains("main"),
            "`main` does not match the filter, so it is not drawn: {rows:?}"
        );
        assert!(
            text.contains("workspace-1"),
            "and the row that does match is: {rows:?}"
        );
    }

    /// The sidebar asks for the compact rows, not the switcher's wider ones (interface spec
    /// 5.5). Both fit 38 columns, so only the row count separates them.
    #[test]
    fn the_sidebar_asks_for_the_compact_rows() {
        let mut compact = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut compact, |_, _| {});
        let drawn = box_rows(&compact);
        let lines = drawn.iter().filter(|r| !r.is_empty()).count();
        assert_eq!(
            lines, 3,
            "a project header and its two workspaces, one line each: {drawn:?}"
        );
    }
}
