//! The sidebar: 38 columns down the left of the screen holding the Projects box over the
//! Agents box, with the hint row under them both (domain model, section 3.6).
//!
//! The project rows come from `render::projects_box`, which the switcher draws from too, and
//! the agent rows from `render::agents_box`, which the agents overlay draws from, so neither
//! list can read one way here and another way there.

use crate::render::agents_box::{self, RowForm};
use crate::render::boxed::{put, put_within};
use crate::render::list_box::{content_width, filter_rows, text_area, ListBox, SIDEBAR_PAD};
use crate::render::projects_box::{filled_index, rows, Extras, PROJECTS_TITLE};
use crate::render::top_bar::Piece;
use crate::render::{theme, RenderInput};
use domux_core::model::{ClientView, Focus, RegionKind, SIDEBAR_WIDTH};
use domux_core::text::truncate_with_ellipsis;
use domux_term::Size;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

/// The bottom row of the sidebar's column: the keys, or the last result.
pub const HINT_ROW_HEIGHT: u16 = 1;

/// The rows the Projects box keeps when the column is too short to halve: a border, a project
/// header and `main`, with a row to spare (plan assumption 21).
pub const MIN_PROJECTS: u16 = 6;

/// And the rows the Agents box keeps: a border and one two-line agent row, with a row to
/// spare.
pub const MIN_AGENTS: u16 = 5;

/// The sidebar's whole column, the full height of the screen.
pub fn sidebar_area(size: Size) -> Rect {
    Rect::new(0, 0, SIDEBAR_WIDTH.min(size.cols), size.rows)
}

/// The sidebar's column, split into the Projects box, the Agents box and the hint row, with
/// one row between the two boxes (interface spec 12.18) and the hint row under both.
///
/// **The Projects box takes the rows its projects need, up to half the column** (MUX-18). It
/// was half the column always, and a reader with four projects had eight blank rows inside a
/// box that could not grow while the Agents box below it scrolled. `wanted` is the height its
/// rows ask for; everything left over is the Agents box's, which is the list that grows during
/// a day's work. The cap at half is what keeps a long project list from squeezing the Agents
/// box down to one row: past half, Projects scrolls like any other list.
///
/// Two floors under that. Projects never goes below `MIN_PROJECTS`, so an empty list still
/// shows its own empty text. And on a column too short for both minimums the Agents box keeps
/// its five rows and Projects takes what is left, because an Agents box squeezed below its
/// minimum has no room for a row at all where a short Projects box still shows a project.
pub fn split_column(column: Rect, wanted: u16) -> (Rect, Rect, Rect) {
    let hint = Rect::new(
        column.x,
        column.y + column.height.saturating_sub(HINT_ROW_HEIGHT),
        column.width,
        HINT_ROW_HEIGHT.min(column.height),
    );
    // What is left for the two boxes: the hint row, and the one row between them.
    let boxes = column.height.saturating_sub(HINT_ROW_HEIGHT + 1);
    let projects_height = wanted
        .min(boxes.div_ceil(2))
        .max(MIN_PROJECTS)
        .min(boxes.saturating_sub(MIN_AGENTS));
    let projects = Rect::new(column.x, column.y, column.width, projects_height);
    let agents = Rect::new(
        column.x,
        column.y + projects_height + 1,
        column.width,
        // Saturating, though the clamp above already bounds `projects_height` by `boxes`.
        // That argument rests on the order of two clamps, and reordering them would panic in
        // debug rather than draw something odd, so it is not an argument worth resting on.
        boxes.saturating_sub(projects_height),
    );
    (projects, agents, hint)
}

/// The height the Projects box asks for: the lines its rows come to, plus its pads and its two
/// borders.
///
/// The whole list and no filter, so the boundary between the two boxes does not move while the
/// reader types in either of them. `/` shortens what is in the box, not the box.
pub fn wanted_projects_height(
    model: &domux_core::model::Model,
    facts: &crate::facts::FactRegistry,
    width: u16,
) -> u16 {
    let built = rows(
        model,
        facts,
        "",
        None,
        Extras::compact(content_width(width, SIDEBAR_PAD)),
    );
    let lines = built
        .rows
        .iter()
        .fold(0u16, |sum, r| sum.saturating_add(r.height()));
    crate::render::list_box::box_lines(lines, SIDEBAR_PAD).saturating_add(2)
}

/// This client's three rectangles: `split_column` with the height the Projects box asks for
/// already worked out.
///
/// Every caller goes through it - the drawing, the pointer and the two handlers that walk these
/// rows - so nobody measures the split a second way (principle 14).
pub fn split_for(
    model: &domux_core::model::Model,
    facts: &crate::facts::FactRegistry,
    size: Size,
) -> (Rect, Rect, Rect) {
    let column = sidebar_area(size);
    split_column(column, wanted_projects_height(model, facts, column.width))
}

/// The Projects box: the first of `split_for`'s rectangles.
pub fn projects_area(
    model: &domux_core::model::Model,
    facts: &crate::facts::FactRegistry,
    size: Size,
) -> Rect {
    split_for(model, facts, size).0
}

/// Which box a pane occupying `pane`'s rows enters on `C-h`: the Agents box when the pane
/// starts beside it, and the Projects box otherwise (interface spec 12.29).
///
/// Every rectangle is in screen rows. `render::workpanel_of` places a pane's rectangle on the
/// screen and `sidebar_area` starts at the screen's own origin, so the two are already
/// measured against the same top edge.
///
/// **Projects is the default and the pane's own top row is the only thing that moves it.** A
/// pane filling the workpanel starts at row 1, beside the Projects box, so it enters Projects
/// at every screen height: interface spec 12.29's own frame 8.2 shows that press landing there,
/// and Projects is the surface `C-h` is reached for. Only a pane that begins at or below the
/// Agents box's first row - the lower pane of a vertical split, which is the case the spec is
/// titled for - is beside the lower box, and that pane enters it.
///
/// **This replaced a comparison of overlapping rows** when MUX-18 let the Projects box shrink
/// to its content. That rule asked which box a pane overlapped more, and it answered Projects
/// for a full-height pane only while the two boxes were equal halves and the row between them
/// was counted with Projects; it leaned on a round-up in `split_column` and on the parity of
/// the screen, and both arguments died with the equal halves. A pane's top row is a fact about
/// the pane rather than about how tall the boxes happen to be drawn today, so the answer no
/// longer moves when a project is added or removed.
pub fn region_for_rows(pane: Rect, agents: Rect) -> RegionKind {
    if pane.y >= agents.y {
        RegionKind::SidebarAgents
    } else {
        RegionKind::SidebarProjects
    }
}

/// Which of the sidebar's two boxes holds the keys, or `None` when they are anywhere else.
///
/// Asked once here rather than matched at each of the five places that need it, so the boxes
/// cannot come to disagree about what "focused" means.
fn focused_box(view: &ClientView) -> Option<RegionKind> {
    match view.focus {
        Focus::Region(kind) if kind.is_sidebar() => Some(kind),
        _ => None,
    }
}

/// The rows the Projects box is showing, and whether the box has the keys.
///
/// The drawing asks, and so does the pointer: the row a reader clicks is the row they see only
/// while the two build one list. Both halves of the filter rule live here for the same reason.
fn built_rows(input: &RenderInput, area: Rect) -> (crate::render::projects_box::Rows, bool) {
    let focused = focused_box(input.view) == Some(RegionKind::SidebarProjects);
    // The fill marks the row the keys act on: the cursor while focus is in the box, and the
    // workspace this client is in otherwise (domain model, section 3.3). `rows` is given the
    // key and hands back where it put the fill, so the band and the brightening cannot land
    // on different rows.
    let key = match focused {
        true => input.view.projects_cursor.as_ref().map(|w| w.as_str()),
        false => None,
    }
    .unwrap_or(input.view.workspace.as_str());
    // The filter is the box's and the box has it only while it has the keys. A box the
    // reader has left draws its whole list: a shortened one with nothing on the screen to say
    // why would be a mode with no marker (principle 2), and the hint row below has no room to
    // carry both the keys and a filter. `api::focus::enter_sidebar_box` starts a fresh
    // filter on the way in, so the two halves of the rule meet.
    let filter = match focused {
        true => input.view.filter.as_str(),
        false => "",
    };
    let built = rows(
        input.model,
        input.facts,
        filter,
        Some(key),
        Extras::compact(content_width(area.width, SIDEBAR_PAD)),
    );
    (built, focused)
}

/// The workspace whose row is at `row` of the screen, or `None` for a header, a blank, the box's
/// border, the hint row, or a row past the end of the list.
pub fn workspace_at(input: &RenderInput, row: u16) -> Option<domux_core::ids::WorkspaceId> {
    let area = projects_area(input.model, input.facts, input.view.size);
    let (built, _) = built_rows(input, area);
    let inner = text_area(area, SIDEBAR_PAD);
    let scroll = crate::render::list_box::scroll_to_show(
        &built.rows,
        built.filled,
        inner.height,
        input.view.projects_scroll,
    );
    let at = crate::render::list_box::row_at(&built.rows, scroll, inner, row)?;
    built
        .rows
        .get(at)
        .and_then(|r| r.key.clone())
        .map(domux_core::ids::WorkspaceId)
}

/// The two boxes in the sidebar's column, with the hint row under them.
pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    let column = sidebar_area(input.view.size);
    let (projects, agents, hint) = split_for(input.model, input.facts, input.view.size);
    // The column's own rectangle first, the way `overlay::frame` clears before drawing its
    // box. `ListBox` paints its border and the text of each row, and leaves the cells after a
    // row's text as it found them, so the region has to be cleared by whoever owns it. The
    // whole column and not each box: the row between them belongs to nothing else, and
    // `hint_row` clears its own.
    clear(column, buf);
    draw_projects(input, projects, buf);
    draw_agents(input, agents, buf);
    hint_row(input, hint, buf);
}

/// Every cell of `area` back to the terminal's own blank, clipped to the buffer.
fn clear(area: Rect, buf: &mut Buffer) {
    let right = area.x.saturating_add(area.width).min(buf.area.right());
    let bottom = area.y.saturating_add(area.height).min(buf.area.bottom());
    for y in area.y..bottom {
        for x in area.x..right {
            buf[(x, y)].reset();
        }
    }
}

fn draw_projects(input: &RenderInput, area: Rect, buf: &mut Buffer) {
    let (built, focused) = built_rows(input, area);
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
        pad: SIDEBAR_PAD,
    }
    .render(area, buf);
}

/// The Agents box: the same rows the agents overlay draws, in the two-line form the 38
/// columns have room for (interface spec 6.3).
///
/// The fill is the cursor's and only while the box has the keys. The Projects box shows one
/// without the keys because it has something else to mark - the workspace this client is in -
/// and there is no such row here, so a fill in a box the reader has left would be a second
/// focus marker with nothing behind it (principle 2).
fn draw_agents(input: &RenderInput, area: Rect, buf: &mut Buffer) {
    let focused = focused_box(input.view) == Some(RegionKind::SidebarAgents);
    // The filter is the box's and the box has it only while it has the keys, the same rule
    // the Projects box above follows and for the same reason.
    let filter = match focused {
        true => input.view.filter.as_str(),
        false => "",
    };
    let all = agents_box::rows(
        input.agents,
        RowForm::Sidebar,
        content_width(area.width, SIDEBAR_PAD),
    );
    let visible = filter_rows(&all, filter);
    let cursor = match focused {
        true => input.view.agents_cursor.as_ref().map(|id| id.to_string()),
        false => None,
    };
    let filled = filled_index(&visible, cursor.as_deref());
    let empty = agents_box::empty_text(filter, RowForm::Sidebar);
    ListBox {
        title: agents_box::TITLE,
        rows: &visible,
        filled,
        focused,
        scroll: input.view.agents_scroll,
        empty_text: &empty,
        // The same pad the Projects box above it takes: the two boxes share a column, so a
        // row of one starting in a different place from a row of the other would read as an
        // indent nobody meant (decision record 0023).
        pad: SIDEBAR_PAD,
    }
    .render(area, buf);
}

/// The hint row's pieces: the keys of the row the cursor is on while focus is in one of the
/// boxes, and the sidebar's own keys otherwise (interface spec 12.11).
///
/// Three arms in one function, so the row cannot answer one way for one box and another way
/// for the other.
pub fn hint_for(input: &RenderInput) -> Vec<Piece> {
    match focused_box(input.view) {
        // Either box: Enter opens the row under the cursor, and `list.activate` is the action
        // both name. The Agents box said `resume` on an exited row until MUX-22 took the
        // exited records off this surface; there is no row here now that Enter resumes, and a
        // hint row is a description of the box in front of the reader (principle 2). The
        // agents overlay's own exited row still carries the word, drawn from
        // `agents_box::RESUME_WORD`.
        Some(_) => pieces(&[
            (input.keymap.list_key_for("list.activate"), "open"),
            (input.keymap.list_key_for("help"), "more"),
        ]),
        None => pieces(&[
            (input.keymap.hint_for("sidebar.toggle"), "hide"),
            (input.keymap.hint_for("switcher.open"), "search"),
        ]),
    }
}

/// Each `(key, word)` as a key piece and a word piece, joined by ` · `.
///
/// A pair whose key is absent draws nothing at all, because the reader has bound that action
/// to nothing and naming a key that does nothing is worse than saying nothing (principle 3).
/// The joiner counts the pieces already built rather than the pair's place in the list, so a
/// dropped first pair does not leave the row starting with a separator.
fn pieces(pairs: &[(Option<String>, &str)]) -> Vec<Piece> {
    let mut out = Vec::new();
    for (key, word) in pairs
        .iter()
        .filter_map(|(key, word)| key.as_deref().map(|key| (key, word)))
    {
        if !out.is_empty() {
            out.push(Piece::new(" · ", Style::default().fg(theme::SURFACE1)));
        }
        out.push(Piece::new(key, Style::default().fg(theme::BLUE)));
        out.push(Piece::new(
            format!(" {word}"),
            Style::default().fg(theme::OVERLAY0),
        ));
    }
    out
}

/// The pieces `hint_for` names; a pill while one is showing, the filter field while it is
/// open, and a start-up note ahead of them (interface spec 12.11 and 12.12).
pub fn hint_row(input: &RenderInput, area: Rect, buf: &mut Buffer) {
    let word = Style::default().fg(theme::OVERLAY0);
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
    let focused = focused_box(input.view).is_some();
    // The filter has the row while `/` is open, the same words the switcher's footer uses,
    // because a mode with no marker on the screen is a mode the reader cannot see they are in
    // (principle 2). Spelled out rather than looked up for the reason `overlay::footer` gives:
    // no action clears the filter, so there is no binding to read.
    //
    // Over the note and under the pill, which is `overlay::footer`'s order (Task 20). Both
    // are surfaces of one question and answering it twice is how they drift: an open text
    // field with no marker is a principle 2 failure, where a note kept waiting behind one is
    // only late. Reachable, though only over the API: `list.filter` opens the field without a
    // key, and it is a key in a box that clears a note.
    //
    // Two surfaces write these words. They are one sentence of text rather than a rule, and
    // the two rows have nothing else in common - this one is 38 cells wide with the sidebar's
    // background, the other spans the overlay and paints its own - so a shared helper here
    // would be a shared name for two unrelated layouts.
    if focused && input.view.filtering {
        // `put_within` throughout: the text is the reader's own typing and it grows without
        // limit, so the row's last cell is what stops it rather than a budget computed here.
        let last_x = area.x + area.width.saturating_sub(2);
        let mut cx = put_within(buf, area.x + 1, area.y, last_x, "Filter › ", word);
        cx = put_within(
            buf,
            cx,
            area.y,
            last_x,
            &input.view.filter,
            Style::default().fg(theme::TEXT),
        );
        cx = put_within(
            buf,
            cx,
            area.y,
            last_x,
            " ",
            Style::default().add_modifier(Modifier::REVERSED),
        );
        put_within(buf, cx, area.y, last_x, "  esc clear", word);
        return;
    }
    // What the start-up prune took away, over the keys and under both the pill and the
    // filter, the same order the switcher's footer draws these four in (`overlay::footer`).
    if let Some(note) = crate::render::note_line(input.notes) {
        put(
            buf,
            area.x + 1,
            area.y,
            &truncate_with_ellipsis(&note, area.width.saturating_sub(2) as usize),
            Style::default().fg(theme::TEXT),
        );
        return;
    }
    // The keys as configured, never the default spelling (principle 3). `put_within` and not
    // `put`: the sidebar's own right edge stops the row, so a rebinding long enough to
    // overflow 38 columns is cut rather than written across the workpanel beside it.
    let last_x = area.x + area.width.saturating_sub(1);
    let mut cx = area.x + 1;
    for piece in hint_for(input) {
        cx = put_within(buf, cx, area.y, last_x, &piece.text, piece.style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::keymap::Keymap;
    use domux_core::model::agent::AgentState;
    use domux_core::model::{Model, Pill};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn view(cols: u16, rows: u16) -> ClientView {
        ClientView {
            size: Size { cols, rows },
            sidebar_open: true,
            ..crate::testing::client_view()
        }
    }

    /// The whole sidebar drawn into a buffer the caller has already prepared, over a git
    /// project with `main` and one named slot, so the cursor and the current workspace can
    /// be different rows. `tweak` gets the view and the two workspace ids.
    fn draw_two(
        buf: &mut Buffer,
        tweak: impl FnOnce(&mut ClientView, &domux_core::ids::WorkspaceId),
    ) {
        draw_sized(buf, 24, tweak)
    }

    /// `draw_two` on a screen `rows` tall, so the box can be made shorter than its rows.
    fn draw_sized(
        buf: &mut Buffer,
        rows: u16,
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
        let mut v = view(120, rows);
        v.workspace = ws;
        v.tab = tab;
        v.focus = Focus::Pane(pane);
        tweak(&mut v, &slot);
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let facts = crate::facts::FactRegistry::new();
        let agents = crate::render::agents_box::AgentsView::empty(chrono::Local::now());
        let input = RenderInput {
            model: &model,
            panes: &panes,
            agents: &agents,
            view: &v,
            keymap: &keymap,
            facts: &facts,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
            notes: &[],
        };
        draw(&input, buf);
    }

    /// The sidebar drawn over a model with no projects at all.
    fn draw_empty(buf: &mut Buffer) {
        let model = Model::new(7);
        let v = view(120, 24);
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let facts = crate::facts::FactRegistry::new();
        let agents = crate::render::agents_box::AgentsView::empty(chrono::Local::now());
        let input = RenderInput {
            model: &model,
            panes: &panes,
            agents: &agents,
            view: &v,
            keymap: &keymap,
            facts: &facts,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
            notes: &[],
        };
        draw(&input, buf);
    }

    /// The three rectangles the sidebar's column splits into, for a buffer the tests drew a
    /// whole screen into. Derived rather than written out, so a test cannot go on reading the
    /// rows of a box that has moved.
    /// Read off the buffer and not worked out a second time. The two boxes are no longer equal
    /// halves of the column (MUX-18), so their heights depend on the model the caller drew
    /// from, and a helper that recomputed the split from the screen size alone would answer for
    /// a sidebar nobody drew. Each box's top row is the border its title sits in.
    fn boxes_of(buf: &Buffer) -> (Rect, Rect, Rect) {
        let width = SIDEBAR_WIDTH.min(buf.area.width);
        let title_row = |title: &str| {
            (0..buf.area.height)
                .find(|y| {
                    (0..width)
                        .map(|x| buf[(x, *y)].symbol())
                        .collect::<String>()
                        .contains(title)
                })
                .unwrap_or_else(|| panic!("no {title} box was drawn"))
        };
        let top = title_row(PROJECTS_TITLE);
        let agents_y = title_row(agents_box::TITLE);
        let hint_y = buf.area.height - HINT_ROW_HEIGHT;
        (
            Rect::new(0, top, width, agents_y - 1 - top),
            Rect::new(0, agents_y, width, hint_y - agents_y),
            Rect::new(0, hint_y, width, HINT_ROW_HEIGHT),
        )
    }

    /// The rows inside one box, as text, one string per line.
    fn rows_of(buf: &Buffer, area: Rect) -> Vec<String> {
        (area.y + 1..area.y + area.height.saturating_sub(1))
            .map(|y| {
                (area.x + 1..area.x + area.width - 1)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// The Projects box's rows.
    fn box_rows(buf: &Buffer) -> Vec<String> {
        rows_of(buf, boxes_of(buf).0)
    }

    /// The hint row drawn into `area` of a buffer the caller has already prepared.
    fn hint_into(view: &ClientView, area: Rect, buf: &mut Buffer) {
        let model = Model::new(1);
        let facts = crate::facts::FactRegistry::new();
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let agents = crate::render::agents_box::AgentsView::empty(chrono::Local::now());
        let input = RenderInput {
            model: &model,
            facts: &facts,
            panes: &panes,
            agents: &agents,
            view,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
            notes: &[],
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

    /// 38 columns of sidebar the whole height of the screen, split into the two boxes and the
    /// hint row, with one row between them and the hint row at the bottom.
    ///
    /// A Projects box that wants more than half the column is capped at half, which is the
    /// split M2 had at every height and the one the geometry tests below read.
    #[test]
    fn the_sidebar_is_38_columns_split_into_two_boxes_over_the_hint_row() {
        let bar = sidebar_area(Size {
            cols: 120,
            rows: 24,
        });
        assert_eq!((bar.x, bar.y, bar.width, bar.height), (0, 0, 38, 24));
        let (projects, agents, hint) = split_column(bar, 40);
        assert_eq!(
            (projects.x, projects.y, projects.width, projects.height),
            (0, 0, 38, 11)
        );
        assert_eq!(
            (agents.x, agents.y, agents.width, agents.height),
            (0, 12, 38, 11),
            "one row between the boxes (interface spec 12.18)"
        );
        assert_eq!((hint.x, hint.y, hint.width, hint.height), (0, 23, 38, 1));
        // Nothing is left over and nothing overlaps: the three rectangles and the one gap row
        // are the whole column.
        assert_eq!(
            projects.height + 1 + agents.height + hint.height,
            bar.height
        );
    }

    /// MUX-18: a Projects box with a few projects in it takes the rows they need and hands the
    /// rest to the Agents box, rather than sitting at half the column with blank rows in it.
    #[test]
    fn a_short_projects_list_gives_its_spare_rows_to_the_agents_box() {
        let bar = sidebar_area(Size {
            cols: 120,
            rows: 40,
        });
        let (projects, agents, _) = split_column(bar, 12);
        assert_eq!(projects.height, 12, "the rows its projects asked for");
        assert_eq!(agents.y, 13);
        assert_eq!(agents.height, 26, "and everything left over");

        // Under `MIN_PROJECTS` the box keeps its floor, so an empty list still has room for
        // its own empty text.
        let (projects, agents, _) = split_column(bar, 1);
        assert_eq!((projects.height, agents.height), (MIN_PROJECTS, 32));
    }

    /// And a list longer than half the column stops at half. Past that the Projects box
    /// scrolls, because an Agents box squeezed to one row is worse than a Projects box the
    /// reader walks.
    #[test]
    fn a_long_projects_list_stops_at_half_the_column() {
        let bar = sidebar_area(Size {
            cols: 120,
            rows: 40,
        });
        let (projects, agents, _) = split_column(bar, 200);
        assert_eq!((projects.height, agents.height), (19, 19));
    }

    /// The height the box asks for is its rows plus its two borders, taken from the whole list
    /// and never from a filtered one: `/` shortens what is in the box, not the box.
    #[test]
    fn the_wanted_height_is_the_rows_and_the_two_borders() {
        let mut model = Model::new(9);
        let facts = crate::facts::FactRegistry::new();
        // An empty model has no rows at all, and `box_lines` floors that at one.
        assert_eq!(wanted_projects_height(&model, &facts, SIDEBAR_WIDTH), 3);
        model
            .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
            .unwrap();
        // One header and one `main` row.
        assert_eq!(wanted_projects_height(&model, &facts, SIDEBAR_WIDTH), 4);
        model
            .add_git_project(PathBuf::from("/repo/domux"), "main".into())
            .unwrap();
        // A blank row before the second header, then its header and its `main`.
        assert_eq!(wanted_projects_height(&model, &facts, SIDEBAR_WIDTH), 7);
    }

    /// A column too short to give both boxes their minimum keeps the Agents box's five rows
    /// and shortens Projects: an Agents box below five has no room for an agent row at all,
    /// where a short Projects box still shows a project (plan assumption 21).
    #[test]
    fn a_column_too_short_for_both_minimums_keeps_the_agents_boxs_rows() {
        let (projects, agents, hint) = split_column(Rect::new(0, 0, 38, 10), 40);
        assert_eq!((projects.height, agents.height), (3, 5));
        assert_eq!(agents.y, projects.height + 1);
        assert_eq!(hint.y, 9);

        // And a column with room for both takes half each, with `MIN_PROJECTS` binding just
        // above the crossover.
        let (projects, agents, _) = split_column(Rect::new(0, 0, 38, 13), 40);
        assert_eq!((projects.height, agents.height), (6, 5));
        let (projects, agents, _) = split_column(Rect::new(0, 0, 38, 40), 40);
        assert_eq!((projects.height, agents.height), (19, 19));
    }

    /// A pane filling the workpanel enters Projects at every screen height the program can be
    /// on, whatever the two boxes are sized to.
    ///
    /// A sweep and not one example. The defect this replaces was not a test that could not
    /// fail; it was a suite whose fixtures never reached the failing case: the overlap rule
    /// this replaced answered Projects only at even screen heights, and every fixture was even.
    /// It walks the whole reachable range instead, and reports the pairs that disagree rather
    /// than stopping at the first. It sweeps the Projects box's own height too, because MUX-18
    /// made that a second thing the answer could turn on and the rule's whole point is that it
    /// does not.
    #[test]
    fn a_pane_filling_the_workpanel_enters_projects_at_every_height() {
        // `render::workpanel_of`: the tab row takes screen row 0, so the one pane of an
        // unsplit tab is rows 1..height.
        let pane = |height: u16| Rect::new(39, 1, 81, height - 1);
        let region = |height: u16, wanted: u16| {
            let (_, agents, _) = split_column(Rect::new(0, 0, SIDEBAR_WIDTH, height), wanted);
            region_for_rows(pane(height), agents)
        };

        // 10 is `render::MIN_ROWS`; below it `compose` draws the notice screen instead.
        let wrong: Vec<(u16, u16)> = (10..=40)
            .flat_map(|h| (1..=40).map(move |w| (h, w)))
            .filter(|(h, w)| region(*h, *w) != RegionKind::SidebarProjects)
            .collect();
        assert!(
            wrong.is_empty(),
            "these (height, wanted) pairs send the one pane of an unsplit tab to the Agents box: {wrong:?}"
        );
    }

    /// A column with no rows at all asks for no rectangle outside itself. Nothing in the
    /// running program passes one - `compose` draws the notice screen below `MIN_ROWS` - but
    /// `Rect` arithmetic on `u16` wraps rather than failing, and the boxes index the buffer
    /// directly.
    #[test]
    fn a_column_with_no_height_splits_into_nothing() {
        let (projects, agents, hint) = split_column(Rect::new(0, 0, 38, 0), 40);
        assert_eq!((projects.height, agents.height, hint.height), (0, 0, 0));
        assert_eq!(projects.y, 0);
    }

    /// `C-h` enters the Agents box only from a pane that starts beside it (interface spec
    /// 12.29).
    ///
    /// All three geometries: a pane high in the screen enters Projects, a pane low in it enters
    /// Agents (the case the spec names), and a pane filling the workpanel enters Projects.
    #[test]
    fn the_box_a_pane_enters_is_the_one_it_starts_beside() {
        let (projects, agents, _) = split_column(Rect::new(0, 0, 38, 30), 40);
        assert_eq!((projects.y, projects.height), (0, 14));
        assert_eq!((agents.y, agents.height), (15, 14));

        let high = Rect::new(39, 1, 81, 14);
        assert_eq!(region_for_rows(high, agents), RegionKind::SidebarProjects);
        let low = Rect::new(39, 15, 81, 15);
        assert_eq!(region_for_rows(low, agents), RegionKind::SidebarAgents);
        let whole = Rect::new(39, 1, 81, 29);
        assert_eq!(
            region_for_rows(whole, agents),
            RegionKind::SidebarProjects,
            "the one pane of an unsplit tab, which covers both boxes and starts beside Projects"
        );
        // A pane that reaches well into the Agents box but starts above it is still beside
        // Projects where it begins. The overlap rule this replaced answered Agents here.
        assert_eq!(
            region_for_rows(Rect::new(39, 14, 81, 16), agents),
            RegionKind::SidebarProjects
        );
        // The Agents box's own first row is the Agents box.
        assert_eq!(
            region_for_rows(Rect::new(39, 15, 81, 1), agents),
            RegionKind::SidebarAgents
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
        // `@` because nothing *this fixture* draws can produce it: the project is
        // `audrey-app`, its workspaces are `main` and `workspace-1`, and there are no facts.
        // Not "nothing the box can draw" - a project path or a branch name containing `@`
        // would put one on the screen. The narrow claim is the one the test rests on, and
        // the one to keep true if the fixture changes.
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

    /// The text of the row carrying the fill inside the Projects box, or `None` if no row
    /// does.
    fn filled_row(buf: &Buffer) -> Option<String> {
        let area = boxes_of(buf).0;
        (area.y + 1..area.y + area.height.saturating_sub(1))
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
            Some("   main"),
            "the keys are in a pane, so the fill stays on the workspace this client is in"
        );

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |v, slot| {
            v.projects_cursor = Some(slot.clone());
            v.focus = Focus::Region(RegionKind::SidebarProjects);
        });
        assert_eq!(
            filled_row(&buf).as_deref(),
            Some("   workspace-1"),
            "with the keys in the box the fill follows the cursor"
        );
    }

    /// The filter drops the rows it does not match, while the box has the keys.
    ///
    /// The focus is set beside the filter because the box applies its filter only while it
    /// holds the keys: a shortened list under a hint row showing `leader b hide` would be a
    /// mode with no marker on the screen (principle 2), and there is a second test below for
    /// that half.
    #[test]
    fn the_filter_drops_the_rows_it_does_not_match() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |v, _| {
            v.filter = "workspace-1".into();
            v.focus = Focus::Region(RegionKind::SidebarProjects);
        });
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

    /// The other half: a box the reader has left draws its whole list again.
    ///
    /// `api::focus::enter_sidebar_box` clears the filter on the way back in, so the field is
    /// never applied by a box that did not take the text; this is what makes that true for
    /// the window between leaving and returning.
    #[test]
    fn the_filter_is_ignored_while_the_keys_are_not_in_the_box() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |v, _| v.filter = "workspace-1".into());
        let rows = box_rows(&buf);
        let text = rows.join("|");
        assert!(
            text.contains("main") && text.contains("workspace-1"),
            "the keys are in a pane, so every row is drawn: {rows:?}"
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

    /// The box is marked focused only while the keys are in it.
    ///
    /// `focused` is the third field `draw` reads that nothing in M2 writes: focus reaches
    /// `RegionKind::SidebarProjects` only through `api::focus::region`, which refuses that
    /// region today, so every test through the harness leaves it false and a box that
    /// hard-coded either answer would look right.
    #[test]
    fn the_box_is_marked_focused_only_while_the_keys_are_in_it() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |_, _| {});
        assert_eq!(
            buf[(0u16, 0u16)].fg,
            ratatui::style::Color::Rgb(0x58, 0x5b, 0x70),
            "the keys are in a pane, so the box takes the unfocused border"
        );

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_two(&mut buf, |v, _| {
            v.focus = Focus::Region(RegionKind::SidebarProjects)
        });
        assert_eq!(
            buf[(0u16, 0u16)].fg,
            ratatui::style::Color::Rgb(0xcb, 0xa6, 0xf7),
            "with the keys in the box it is the focused region, so the border is the accent"
        );
    }

    /// The scroll the client remembers moves the view.
    ///
    /// The fourth no-writer field: `projects_scroll` is only ever set by list navigation,
    /// which M2 has not built. The screen here is short enough that the box cannot show
    /// every row, which is the only condition under which the field can matter at all.
    ///
    /// 11 rows and not 12. The column splits between the two boxes from M3, and the Agents
    /// box keeps its five rows on a column too short for both minimums, so 11 leaves the
    /// Projects box two lines for this fixture's three and 12 leaves it exactly three. At 12
    /// a hard-coded scroll of 0 passes the test.
    #[test]
    fn the_scroll_the_client_remembers_moves_the_view() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 11));
        draw_sized(&mut buf, 11, |_, _| {});
        let top: String = (1..37u16).map(|x| buf[(x, 1u16)].symbol()).collect();
        assert!(
            top.starts_with(" AUDREY-APP"),
            "unscrolled, the box starts at the project header: {top:?}"
        );

        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 11));
        draw_sized(&mut buf, 11, |v, _| v.projects_scroll = 1);
        let top: String = (1..37u16).map(|x| buf[(x, 1u16)].symbol()).collect();
        assert_eq!(
            top.trim_end(),
            "   main",
            "scrolled by one, the header has moved off the top"
        );
    }

    /// One agent, in the state the caller asks for, with the resume key the keymap gives.
    fn agents_view(state: AgentState) -> crate::render::agents_box::AgentsView {
        let mut view = crate::render::agents_box::AgentsView::empty(chrono::Local::now());
        view.agents.push(crate::render::agents_box::AgentEntry {
            id: domux_core::ids::AgentId("a_0001".into()),
            kind: domux_core::model::agent::AgentKind::Claude,
            name: None,
            state,
            unseen: false,
            recap: Some("read the transcript".into()),
            project: "audrey-app".into(),
            place_with_tab: "audrey-app › main › pr1".into(),
            place_without_tab: "audrey-app › main".into(),
            place_in_project: "main › pr1".into(),
            last_activity_at: "2026-09-04T14:30:00+01:00".into(),
            word: "",
        });
        view.resume_key = Keymap::defaults().list_key_for("list.activate");
        view
    }

    /// The hint row over a view with one agent in `state`, the cursor on it or not.
    fn agents_hint(state: AgentState, on_the_cursor: bool) -> String {
        agents_hint_with(agents_view(state), on_the_cursor)
    }

    /// The hint row over an `AgentsView` the caller has built, so a test can hand it a field
    /// the running program would not produce.
    fn agents_hint_with(
        agents: crate::render::agents_box::AgentsView,
        on_the_cursor: bool,
    ) -> String {
        let mut v = view(120, 24);
        v.focus = Focus::Region(RegionKind::SidebarAgents);
        if on_the_cursor {
            v.agents_cursor = Some(agents.agents[0].id.clone());
        }
        let model = Model::new(1);
        let facts = crate::facts::FactRegistry::new();
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let input = RenderInput {
            model: &model,
            facts: &facts,
            panes: &panes,
            agents: &agents,
            view: &v,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
            notes: &[],
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 38, 1));
        hint_row(&input, Rect::new(0, 0, 38, 1), &mut buf);
        line_of(&buf, 0)
    }

    /// Every row in the sidebar's Agents box offers `open`, because every record in it is
    /// running (MUX-22). The word was `resume` on an exited row until the exited records left
    /// this surface, and the agents overlay's own exited row carries it now.
    #[test]
    fn the_hint_row_names_open_for_every_row_in_the_sidebars_box() {
        assert_eq!(
            agents_hint(AgentState::Working, true),
            " ⏎ open · ? more                      "
        );
        assert_eq!(
            agents_hint(AgentState::Exited, true),
            " ⏎ open · ? more                      ",
            "an exited record has no row here for the cursor to be on"
        );
        assert_eq!(
            agents_hint(AgentState::Exited, false),
            " ⏎ open · ? more                      "
        );
    }

    /// An action the reader has bound to nothing drops out of the row rather than naming a
    /// key that does nothing, and the row does not start with the separator that pair would
    /// have carried (principle 3).
    ///
    /// Unreachable through `Keymap::defaults`, which binds both, so the pieces are built
    /// directly. The old row built its separator from the pair's place in the list, where a
    /// dropped first pair left a leading ` · `.
    #[test]
    fn a_pair_whose_key_is_unbound_draws_nothing_and_leaves_no_separator() {
        let drawn: String = pieces(&[(None, "open"), (Some("?".into()), "more")])
            .iter()
            .map(|p| p.text.clone())
            .collect();
        assert_eq!(drawn, "? more");
        assert!(pieces(&[(None, "open"), (None, "more")]).is_empty());
    }

    /// The Agents box shows the fill on the cursor only while it has the keys, and applies
    /// the filter only then too: a box the reader has left is showing its whole list under a
    /// hint row that says nothing about a filter, and a second fill would be a second focus
    /// marker (principle 2).
    #[test]
    fn the_agents_box_takes_its_cursor_and_its_filter_only_while_it_has_the_keys() {
        let agents = agents_view(AgentState::Working);
        let id = agents.agents[0].id.clone();
        let draw_into = |focused: bool, filter: &str| {
            let mut v = view(120, 24);
            v.agents_cursor = Some(id.clone());
            v.filter = filter.into();
            if focused {
                v.focus = Focus::Region(RegionKind::SidebarAgents);
            }
            let model = Model::new(1);
            let facts = crate::facts::FactRegistry::new();
            let panes = HashMap::new();
            let keymap = Keymap::defaults();
            let input = RenderInput {
                model: &model,
                facts: &facts,
                panes: &panes,
                agents: &agents,
                view: &v,
                keymap: &keymap,
                now: chrono::Local::now(),
                config_error: None,
                hint: None,
                notes: &[],
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
            draw(&input, &mut buf);
            buf
        };

        let area = boxes_of(&draw_into(true, "")).1;
        let filled = |buf: &Buffer| {
            (area.y + 1..area.y + area.height - 1)
                .any(|y| buf[(2u16, y)].bg != ratatui::style::Color::Reset)
        };
        assert!(
            filled(&draw_into(true, "")),
            "with the keys in the box the cursor row carries the fill"
        );
        assert!(
            !filled(&draw_into(false, "")),
            "and with the keys in a pane no row does"
        );

        let rows = rows_of(&draw_into(true, "codex"), area);
        assert!(
            rows.iter().all(|r| !r.contains("claude")),
            "the filter drops the row it does not match: {rows:?}"
        );
        let rows = rows_of(&draw_into(false, "codex"), area);
        assert!(
            rows.iter().any(|r| r.contains("claude")),
            "and a box the reader has left draws its whole list again: {rows:?}"
        );
    }

    /// With no projects the box names the state and the next action (principle 9).
    ///
    /// The fifth no-writer field: `empty_text` is unreachable through the harness, which
    /// always starts with a project, so a box that dropped it entirely would pass.
    #[test]
    fn an_empty_projects_box_names_the_command_that_fills_it() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        draw_empty(&mut buf);
        let text = box_rows(&buf).join(" ");
        assert!(
            text.contains("No projects yet"),
            "it says there are none: {text:?}"
        );
        assert!(
            text.contains("domux2 open"),
            "and names the command that adds one: {text:?}"
        );
    }
}
