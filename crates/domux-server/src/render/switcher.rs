//! The switcher: one overlay holding the Projects box and its footer (`leader s`).
//!
//! The box is the sidebar's box, from the same row builder, so a project cannot read one way
//! in the sidebar and another here (domain model, section 3.6). What the switcher adds is
//! the width: `Extras::switcher` asks for the pull request title and the tab list, which the
//! sidebar's 38 columns have no room for.

use crate::render::list_box::ListBox;
use crate::render::projects_box::{rows, Extras, PROJECTS_TITLE};
use crate::render::{overlay, RenderInput};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The footer's keys, in reading order: what Enter does, then the filter, then help, then
/// the way out. Each is an action, looked up in `[keys.list]` when the row is drawn.
const HINTS: &[(&str, &str)] = &[
    ("list.activate", "open"),
    ("list.filter", "filter"),
    ("help", "help"),
    ("focus.pane", "close"),
];

pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    let screen = buf.area;
    // Two passes. The rows truncate to the box's inner width, and the row count then decides
    // the box's height; the width answers first because it does not depend on the rows.
    let inner_width = overlay::list_overlay_width(screen).saturating_sub(2);
    // The cursor carries the fill while focus is in the box; with no cursor it is the
    // workspace this client is in (domain model, section 3.3).
    let filled = input
        .view
        .projects_cursor
        .as_ref()
        .map(|w| w.as_str())
        .unwrap_or_else(|| input.view.workspace.as_str());
    let rows = rows(
        input.model,
        input.facts,
        &input.view.filter,
        Some(filled),
        Extras::switcher(inner_width),
    );
    let lines = rows
        .rows
        .iter()
        .fold(0u16, |sum, r| sum.saturating_add(r.height()));
    let area = overlay::list_overlay_area(screen, lines);
    let footer = Rect::new(area.x, area.y + area.height, area.width, 1);
    let empty = empty_text(input);
    // `clear` and not `frame_at`: a `ListBox` draws its own `Boxed`, so the switcher paints
    // the background and lets the box own the border.
    overlay::clear(area, buf);
    ListBox {
        title: PROJECTS_TITLE,
        rows: &rows.rows,
        filled: rows.filled,
        focused: true,
        scroll: input.view.projects_scroll,
        empty_text: &empty,
    }
    .render(area, buf);
    overlay::footer(input, HINTS, footer, buf);
    // Last, so that what is dimmed is what the switcher did not draw. The corrected scroll
    // `render` returns is dropped on purpose: a renderer does not write to the model, and
    // `list.down` and `list.up` own the scroll (Task 14).
    overlay::dim(buf, &[area, footer]);
}

/// What the box says when the rows are empty: the state, and the next action (principle 9).
fn empty_text(input: &RenderInput) -> String {
    if input.view.filter.trim().is_empty() {
        format!(
            "No projects yet. Add one with {} open <path>",
            domux_core::names::BIN_NAME
        )
    } else {
        format!(
            "No workspace matches {:?}. esc clears the filter",
            input.view.filter.trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::FactRegistry;
    use domux_core::ids::ProjectId;
    use domux_core::ids::{ClientId, PaneId, TabId, WorkspaceId};
    use domux_core::keymap::Keymap;
    use domux_core::model::{
        ClientView, Focus, Model, Project, ProjectKind, TextInput, Workspace, WorkspaceHandle,
    };
    use domux_core::proto::Capabilities;
    use domux_term::Size;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::collections::HashMap;

    fn view(filter: &str) -> ClientView {
        ClientView {
            id: ClientId("c_0001".into()),
            size: Size { cols: 80, rows: 24 },
            caps: Capabilities::default(),
            workspace: WorkspaceId("w_0001".into()),
            tab: TabId("t_0001".into()),
            focus: Focus::Pane(PaneId("p_0001".into())),
            sidebar_open: false,
            overlay: None,
            chord: None,
            filter: filter.into(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            filtering: false,
            input: TextInput::new(""),
            overlay_under: None,
            pill: None,
        }
    }

    /// One project holding `main` and four slots, none of them with a tab or a fact, so every
    /// workspace row is exactly one line and the row a line number lands on can be counted.
    fn model_with_slots() -> Model {
        let mut model = Model::new(1);
        let workspace = |handle: WorkspaceHandle, id: &str| Workspace {
            id: WorkspaceId(id.into()),
            handle,
            name: None,
            path: "/p".into(),
            tabs: Vec::new(),
            last_tab: None,
        };
        let mut workspaces = vec![workspace(WorkspaceHandle::Main, "w_main")];
        for n in 1..=4u32 {
            workspaces.push(workspace(WorkspaceHandle::Slot(n), &format!("w_{n}")));
        }
        model.projects.push(Project {
            id: ProjectId("p_1".into()),
            name: "proj".into(),
            root: "/p".into(),
            kind: ProjectKind::Folder,
            workspaces,
        });
        model
    }

    fn draw_into(model: &Model, view: &ClientView, cols: u16, rows: u16) -> Buffer {
        let facts = FactRegistry::new();
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let input = RenderInput {
            model,
            facts: &facts,
            panes: &panes,
            view,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, cols, rows));
        draw(&input, &mut buf);
        buf
    }

    /// The text of one row of the box's inside, at the 80 column geometry.
    fn inner_line(buf: &Buffer, y: u16) -> String {
        (11..69)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// The fill follows the cursor, not the workspace the client is in. Nothing outside this
    /// file can yet move the cursor off the current workspace - `list.down` is Task 14 - so
    /// the two are equal in every harness test, and only a fixture that separates them can
    /// tell which one the box actually read.
    #[test]
    fn the_fill_follows_the_cursor_and_not_the_workspace_the_client_is_in() {
        let model = model_with_slots();
        let mut v = view("");
        v.workspace = WorkspaceId("w_main".into());
        v.projects_cursor = Some(WorkspaceId("w_2".into()));
        let buf = draw_into(&model, &v, 80, 24);
        // Rows inside the box: 4 header, 5 main, 6 blank, 7 workspace-1, 8 blank, 9
        // workspace-2.
        assert_eq!(inner_line(&buf, 9), "workspace-2");
        assert_eq!(buf[(11, 9)].bg, crate::render::theme::SURFACE0);
        assert_eq!(inner_line(&buf, 5), "main");
        assert_ne!(
            buf[(11, 5)].bg,
            crate::render::theme::SURFACE0,
            "and the workspace this client is in does not also carry one"
        );
    }

    /// The box starts at the line the client remembers, so a list that has been scrolled
    /// stays where the reader left it rather than jumping to the smallest view that holds
    /// the cursor.
    #[test]
    fn the_box_starts_at_the_remembered_scroll_when_the_cursor_is_already_in_view() {
        let model = model_with_slots();
        let mut v = view("");
        v.size = Size { cols: 80, rows: 14 };
        v.workspace = WorkspaceId("w_main".into());
        v.projects_cursor = Some(WorkspaceId("w_3".into()));
        // Ten lines of rows into six rows of box. Line 7 is workspace-3, so a view starting
        // at line 3 holds it and is kept; a box that ignored the scroll would correct 0 up
        // to 2, the least it can be with workspace-3 in view.
        v.projects_scroll = 3;
        let buf = draw_into(&model, &v, 80, 14);
        assert_eq!(inner_line(&buf, 4), "workspace-1");
        assert_eq!(inner_line(&buf, 8), "workspace-3");
        assert_eq!(buf[(11, 8)].bg, crate::render::theme::SURFACE0);
    }

    /// The switcher over a model with no projects at all, which is the only way to reach the
    /// empty box: the server seeds a project from the directory it was started in, so no
    /// harness test can get there.
    fn empty_model_row(filter: &str) -> String {
        let model = Model::new(1);
        let facts = FactRegistry::new();
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let view = view(filter);
        let input = RenderInput {
            model: &model,
            facts: &facts,
            panes: &panes,
            view: &view,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
        };
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 80, 24));
        draw(&input, &mut buf);
        (0..80).map(|x| buf[(x, 4)].symbol()).collect()
    }

    /// An empty box names the state and the next action, in the words of the binary the
    /// reader actually typed (principle 9).
    #[test]
    fn an_empty_switcher_says_how_to_add_a_project() {
        assert_eq!(
            empty_model_row("").trim_end(),
            "          │No projects yet. Add one with domux2 open <path>          │"
        );
    }

    /// A filter that matches nothing says so and says how to get back, rather than showing
    /// the same empty box as a domux with no projects in it.
    #[test]
    fn a_filter_that_matches_nothing_says_what_was_searched_for() {
        assert_eq!(
            empty_model_row("  auth  ").trim_end(),
            "          │No workspace matches \"auth\". esc clears the filter        │"
        );
    }
}
