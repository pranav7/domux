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
    use domux_core::facts::{Fact, FactKey, FACT_BRANCH};
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
            sidebar_forced: false,
            overlay: None,
            chord: None,
            filter: filter.into(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            agents_cursor: None,
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
        draw_with_facts(model, &FactRegistry::new(), view, cols, rows)
    }

    fn draw_with_facts(
        model: &Model,
        facts: &FactRegistry,
        view: &ClientView,
        cols: u16,
        rows: u16,
    ) -> Buffer {
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let input = RenderInput {
            model,
            facts,
            panes: &panes,
            view,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
            notes: &[],
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

    /// With no cursor the fill falls back to the workspace this client is in (domain model,
    /// section 3.3), which is the branch `switcher.open` never takes: it always sets the
    /// cursor, so only a fixture that clears the cursor by hand reaches the fallback. The
    /// other four workspaces are there so the fill has somewhere else it could have landed.
    #[test]
    fn the_fill_falls_back_to_the_workspace_this_client_is_in_when_there_is_no_cursor() {
        let model = model_with_slots();
        let mut v = view("");
        v.workspace = WorkspaceId("w_2".into());
        v.projects_cursor = None;
        let buf = draw_into(&model, &v, 80, 24);
        assert_eq!(inner_line(&buf, 9), "workspace-2");
        assert_eq!(buf[(11, 9)].bg, crate::render::theme::SURFACE0);
        assert_eq!(inner_line(&buf, 5), "main");
        assert_ne!(
            buf[(11, 5)].bg,
            crate::render::theme::SURFACE0,
            "and no other row carries one"
        );
    }

    /// `/` narrows the box to the workspaces that match, so a filter reaches the rows
    /// through the switcher and not only through the box's own tests.
    ///
    /// The fixture needs both halves: a filter that matches something and a model holding
    /// rows it does not match. Every other fixture in this file has an empty filter over a
    /// full model or a filter over an empty one, and both of those pass just as well when
    /// the switcher hands the rows an empty filter.
    #[test]
    fn the_box_shows_only_the_workspaces_the_filter_matches() {
        let model = model_with_slots();
        let mut v = view("workspace-2");
        v.workspace = WorkspaceId("w_main".into());
        let buf = draw_into(&model, &v, 80, 24);
        assert!(
            inner_line(&buf, 4).starts_with("PROJ "),
            "the header of the project the match is in stays: {:?}",
            inner_line(&buf, 4)
        );
        assert_eq!(inner_line(&buf, 5), "workspace-2");
        assert_eq!(
            inner_line(&buf, 6),
            "─".repeat(58),
            "and the box is two lines tall, so main and the other three slots are gone"
        );
    }

    /// The rows carry the facts the core holds: a branch reaches line 2, and a branch equal
    /// to the handle leaves the slot untouched and hollow (interface spec 12.23).
    ///
    /// This is the field `RenderInput` gained for this task. Every other fixture here, and
    /// every harness test, renders against an empty registry - `HarnessOptions.providers`
    /// is empty by default - so handing `rows` a fresh registry instead of the core's would
    /// draw the same screen everywhere else.
    #[test]
    fn the_rows_read_the_fact_registry_the_core_passed_in() {
        let model = model_with_slots();
        let mut facts = FactRegistry::new();
        let fact = |text: &str| {
            Fact::new(
                text,
                None,
                "2026-09-05T10:00:00+01:00",
                std::time::Duration::from_secs(600),
            )
        };
        facts.set(
            FactKey::workspace(&WorkspaceId("w_1".into()), FACT_BRANCH),
            Some(fact("workspace-1")),
        );
        facts.set(
            FactKey::workspace(&WorkspaceId("w_2".into()), FACT_BRANCH),
            Some(fact("feat/auth")),
        );
        let mut v = view("");
        v.workspace = WorkspaceId("w_main".into());
        v.projects_cursor = Some(WorkspaceId("w_main".into()));
        let buf = draw_with_facts(&model, &facts, &v, 80, 24);
        assert_eq!(
            inner_line(&buf, 7),
            "◌ workspace-1",
            "a slot on its own branch is untouched"
        );
        assert_eq!(inner_line(&buf, 9), "workspace-2");
        assert_eq!(
            inner_line(&buf, 10),
            "feat/auth",
            "and a branch of its own gets line 2, which is a second line the row did not have"
        );
        assert_eq!(
            inner_line(&buf, 12),
            "workspace-3",
            "so everything under it moved down a row"
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
            notes: &[],
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
