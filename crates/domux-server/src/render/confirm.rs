//! The confirmation overlay: the question a destructive key asks before it acts
//! (interface spec 7.3). Task 16 wrote the project kind's copy and the drawing they share;
//! Task 18 added the workspace kinds'.
//!
//! Every kind draws the same five parts in the same order: the title in the border in red,
//! the identity line that says which object, what goes, what stays, and the two keys. The
//! words come from the handler that would do the work, so the box and the sentence a shell
//! prints are built from one spelling and cannot drift apart.
//!
//! `ConfirmKind::CloseTab` reaches here too, and must stay drawing nothing: M1 asks that
//! question in the tab's own cell (`top_bar::draw`), so a box drawn here would ask it twice.

use crate::render::boxed::put_within;
use crate::render::{overlay, theme, RenderInput};
use domux_core::facts::{FactKey, FACT_BRANCH};
use domux_core::model::ConfirmKind;
use domux_core::text::{display_width, truncate_with_ellipsis};
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// A cell of padding inside the border on each side, and the two border columns.
const CHROME: u16 = 4;

/// What the confirmation says, in the order it is drawn: the title goes in the border and
/// the rest fills the box.
struct Question {
    title: String,
    lines: Vec<Line<'static>>,
}

pub fn draw(input: &RenderInput, kind: &ConfirmKind, buf: &mut Buffer) {
    let Some(question) = question(input, kind) else {
        return;
    };
    let widest = question
        .lines
        .iter()
        .map(|l| l.width())
        .chain(std::iter::once(display_width(&question.title)))
        .max()
        .unwrap_or(0) as u16;
    // The width first, on its own, because the height depends on it: a line too long for the
    // screen becomes two rows, so the rows cannot be counted until the box is as wide as it is
    // going to get. `centred_area` is asked twice for that reason and its width answer does
    // not depend on the height it is passed.
    let wide_enough = overlay::centred_area(widest.saturating_add(CHROME), 3, buf);
    let lines = wrapped(
        question.lines,
        wide_enough.width.saturating_sub(CHROME) as usize,
    );
    let area = overlay::centred_area(wide_enough.width, lines.len() as u16 + 2, buf);
    if area.width == 0 || area.height == 0 {
        return;
    }
    // An empty title, then the question written into the border row here: `Boxed` draws a
    // title in the accent when focused and in `overlay1` when not, and this one is red.
    let inner = overlay::frame_at("", area, buf);
    let right = area.x + area.width - 1;
    put_within(
        buf,
        area.x + 1,
        area.y,
        right,
        &format!(
            " {} ",
            truncate_with_ellipsis(&question.title, area.width.saturating_sub(CHROME) as usize)
        ),
        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
    );
    let width = inner.width.saturating_sub(2) as usize;
    let last_x = inner.x + inner.width.saturating_sub(1);
    for (i, line) in lines.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let mut x = inner.x + 1;
        // One budget for the whole line, spent span by span: giving each span the line's
        // full width would let three short spans measure as fitting and draw past the
        // border, which `put_within` would then clip without an ellipsis to say so.
        //
        // Unpinned, and it cannot be pinned by any fixture here: the box is sized from its
        // widest line, so no line reaches the boundary until `centred_area` clamps the box
        // to a screen narrower than its content. Only the keys line has several spans, and
        // the two versions then differ by one cell - the ellipsis - because `put_within`
        // clips both at the same column. Said here rather than left silent.
        let mut budget = width;
        for span in &line.spans {
            if budget == 0 {
                break;
            }
            let text = truncate_with_ellipsis(&span.content, budget);
            budget = budget.saturating_sub(display_width(&text));
            x = put_within(buf, x, inner.y + i as u16, last_x, &text, span.style);
        }
    }
    // The screen behind reads as being behind it (interface spec 7.1). After the drawing,
    // so the box itself is what `keep` keeps rather than something the box then undoes.
    overlay::dim(buf, &[area]);
}

/// Breaks each line to `width` columns so a sentence too long for the box becomes two rows
/// rather than losing its tail to an ellipsis.
///
/// **Why this is here rather than solved by shorter copy.** The delete box is the tight one:
/// its sentence is a fixed 58 columns plus the branch name, against a budget of 112 at a 120
/// column screen, so it clips at a 55 character branch and fits at 54. Real branch names in
/// this program already reach 53. Shortening the copy moved that ceiling and did not remove
/// it, and a ceiling two characters above what the author types is not a guarantee. What is
/// lost when it clips is the end of the sentence, which on a delete is the branch and the tab
/// count.
///
/// The 58 is `closes N tabs`, which is every count but one; `closes 1 tab` is a column
/// shorter and clips one character later. An earlier version of this paragraph gave 57 beside
/// a threshold of 55, which cannot both be true, and the inconsistency was checkable from the
/// paragraph alone.
///
/// Only a line of one span is broken. A line of several is a keys row, built from short pieces
/// to fit, and breaking it would have to carry each piece's style across the break for no gain.
/// A blank line has no spans at all and passes through as itself, which is what keeps the
/// spacing the box was written with.
///
/// A word longer than the row stays on its own row and `put_within` gives it an ellipsis. That
/// is the worktree path on the identity line, which has no spaces to break on: the deferral is
/// deliberate and measured at roughly 110 characters of path, and the title still names the
/// slot.
fn wrapped(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for line in lines {
        let [span] = &line.spans[..] else {
            out.push(line);
            continue;
        };
        let rows = broken(&span.content, width);
        if rows.is_empty() {
            out.push(Line::default());
            continue;
        }
        for row in rows {
            out.push(Line::from(Span::styled(row, span.style)));
        }
    }
    out
}

/// `text` in rows of at most `width` columns, broken on spaces.
fn broken(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        if row.is_empty() {
            row.push_str(word);
        } else if display_width(&row) + 1 + display_width(word) <= width {
            row.push(' ');
            row.push_str(word);
        } else {
            rows.push(std::mem::take(&mut row));
            row.push_str(word);
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// The copy for one question, or `None` when there is nothing to ask about: a project the
/// model no longer holds, or the tab kind M1 asks about somewhere else.
fn question(input: &RenderInput, kind: &ConfirmKind) -> Option<Question> {
    match kind {
        // M1's, asked in the tab's own cell.
        ConfirmKind::CloseTab(_) => None,
        ConfirmKind::RemoveProject(id) => {
            let project = input.model.project(id)?;
            let copy = crate::api::project::removal_copy(
                &project.name,
                project.workspaces.len(),
                &project.root,
            );
            Some(Question {
                title: copy.title,
                lines: vec![
                    Line::from(Span::styled(
                        copy.identity,
                        Style::default().fg(theme::OVERLAY1),
                    )),
                    Line::default(),
                    Line::from(Span::styled(copy.removes, Style::default().fg(theme::TEXT))),
                    Line::from(Span::styled(
                        crate::api::project::KEEPS_THE_FOLDER.to_string(),
                        Style::default().fg(theme::OVERLAY0),
                    )),
                    Line::default(),
                    keys("y", "remove project", "esc", "keep project"),
                ],
            })
        }
        ConfirmKind::DeleteWorkspace(id) => {
            let w = input.model.workspace(id)?;
            let root = input
                .model
                .project_of_workspace(id)
                .map(|p| p.root.clone())?;
            // What was observed, never the handle. A slot checked out on `feat/auth-cleanup`
            // is asked about by that branch; a slot whose branch fact has not arrived is asked
            // about without one (principle 4).
            //
            // The fact, not a fresh read, because a frame is composed on the core task and no
            // git command may run there. It is what the question promises rather than what the
            // delete will do: the job reads the worktree itself and refuses if the two have
            // come apart. That reconciliation covers this box, because a key is asked and
            // answered in one session; it does not cover a shell, and
            // `CoreJob::DeleteWorkspace::expected_branch` says why and what that costs.
            let branch = input
                .facts
                .get(&FactKey::workspace(id, FACT_BRANCH))
                .map(|f| f.text.clone());
            let copy = crate::api::workspace::deletion_copy(
                &w.display_name(),
                &root,
                &w.path,
                branch.as_deref(),
                w.tabs.len(),
            );
            Some(Question {
                title: copy.title,
                lines: vec![
                    Line::from(Span::styled(
                        copy.identity,
                        Style::default().fg(theme::OVERLAY1),
                    )),
                    Line::default(),
                    Line::from(Span::styled(
                        copy.removes_without_the_path,
                        Style::default().fg(theme::TEXT),
                    )),
                    Line::from(Span::styled(
                        copy.keeps.to_string(),
                        Style::default().fg(theme::OVERLAY0),
                    )),
                    Line::default(),
                    keys("y", "delete workspace", "esc", "keep workspace"),
                ],
            })
        }
        ConfirmKind::ClearWorkspace(id) => {
            let w = input.model.workspace(id)?;
            // No project lookup here, unlike the delete arm: `clear_copy` names no path
            // inside its sentence, so there is nothing to make relative.
            let copy = crate::api::workspace::clear_copy(&w.display_name(), &w.path);
            Some(Question {
                title: copy.title,
                lines: vec![
                    Line::from(Span::styled(
                        copy.identity,
                        Style::default().fg(theme::OVERLAY1),
                    )),
                    Line::default(),
                    Line::from(Span::styled(copy.removes, Style::default().fg(theme::TEXT))),
                    Line::from(Span::styled(
                        copy.keeps.to_string(),
                        Style::default().fg(theme::OVERLAY0),
                    )),
                    Line::from(Span::styled(
                        copy.stops.to_string(),
                        Style::default().fg(theme::OVERLAY0),
                    )),
                    Line::default(),
                    keys("y", "clear workspace", "esc", "keep workspace"),
                ],
            })
        }
    }
}

/// `y remove project    esc keep project`: the key in blue, what it does in `text`, four
/// spaces between the pair (interface spec 7.3).
fn keys(yes: &str, does: &str, no: &str, undoes: &str) -> Line<'static> {
    let key = Style::default().fg(theme::BLUE);
    let word = Style::default().fg(theme::TEXT);
    Line::from(vec![
        Span::styled(yes.to_string(), key),
        Span::styled(format!(" {does}    "), word),
        Span::styled(no.to_string(), key),
        Span::styled(format!(" {undoes}"), word),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::FactRegistry;
    use domux_core::facts::Fact;
    use domux_core::ids::{ClientId, PaneId, ProjectId, TabId, WorkspaceId};
    use domux_core::keymap::Keymap;
    use domux_core::model::{
        ClientView, Focus, Model, Overlay, Project, ProjectKind, TextInput, Workspace,
        WorkspaceHandle,
    };
    use domux_core::proto::Capabilities;
    use domux_term::Size;
    use ratatui::layout::Rect;
    use std::collections::HashMap;
    use std::time::Duration;

    /// A project holding one slot, at a path as long as a real one.
    ///
    /// **The `path` is load-bearing and the `root` is not**, and both halves of that are
    /// measured rather than reasoned. Shortening `path` to `/p/w1` kills two tests here,
    /// because it is what the identity line shows. Putting `root` back to `/p` kills nothing,
    /// because `relative_to` only feeds `DeletionCopy::removes`, which is the long form the
    /// CLI prints and this box never draws; the harness covers that form against real paths.
    /// So the root is a plausible parent for the path rather than a value any test depends on.
    ///
    /// This comment has been wrong twice, which is why it now carries its evidence. It first
    /// said a two-character root would let the width test pass at a width no reader has; the
    /// reviewer disproved that by running it. It then said the root buys the identity line;
    /// that is the `path`. Both were guesses about a fixture, written in the shape of reasons.
    ///
    /// The path and the root are literals and nothing here touches a filesystem: the box is
    /// composed on the core task, so it reads the model and the facts and never asks git.
    fn model_with_a_slot() -> (Model, WorkspaceId) {
        let mut model = Model::new(1);
        let id = WorkspaceId("w_1".into());
        model.projects.push(Project {
            id: ProjectId("p_1".into()),
            name: "audrey-app".into(),
            root: "/Users/pranav/projects/audrey-app".into(),
            kind: ProjectKind::Folder,
            workspaces: vec![Workspace {
                id: id.clone(),
                handle: WorkspaceHandle::Slot(1),
                name: None,
                path: "/Users/pranav/projects/audrey-app/.domux/worktrees/workspace-1".into(),
                tabs: Vec::new(),
                last_tab: None,
            }],
        });
        (model, id)
    }

    fn view(overlay: Overlay) -> ClientView {
        ClientView {
            id: ClientId("c_0001".into()),
            size: Size {
                cols: 160,
                rows: 24,
            },
            caps: Capabilities::default(),
            workspace: WorkspaceId("w_1".into()),
            tab: TabId("t_0001".into()),
            focus: Focus::Pane(PaneId("p_0001".into())),
            sidebar_open: false,
            sidebar_forced: false,
            overlay: Some(overlay),
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

    /// The whole box as text, rows joined by newlines.
    ///
    /// The buffer is filled with `@` first, which is a character nothing this fixture draws
    /// can produce - the paths, the names and the copy here are all spelled out above and
    /// none of them holds one. It is not a claim that the box could never draw an `@`: a
    /// branch or a path containing one would. It is here so the box has something to cover.
    fn drawn(model: &Model, facts: &FactRegistry, kind: &ConfirmKind) -> String {
        drawn_at(model, facts, kind, 160)
    }

    fn drawn_at(model: &Model, facts: &FactRegistry, kind: &ConfirmKind, cols: u16) -> String {
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let mut v = view(Overlay::Confirm(kind.clone()));
        v.size = Size { cols, rows: 24 };
        let input = RenderInput {
            model,
            facts,
            panes: &panes,
            view: &v,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, cols, 24));
        for cell in buf.content.iter_mut() {
            cell.set_symbol("@");
        }
        // Through `overlay::draw` rather than straight into `draw`, so the arm that routes a
        // `Confirm` here is on the far side of the test too.
        crate::render::overlay::draw(&input, &mut buf);
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn branch_fact(text: &str) -> Fact {
        Fact::new(
            text,
            None,
            chrono::Local::now().to_rfc3339(),
            Duration::from_secs(600),
        )
    }

    /// The box names the branch the branch provider observed, not the one the handle is named
    /// after, and it draws over what was underneath it.
    ///
    /// `feat/auth-cleanup` rather than `workspace-1`, because a slot whose branch equals its
    /// handle cannot tell the fact from the handle: both would spell the same line, and a box
    /// that never read the registry would pass.
    #[test]
    fn the_delete_box_names_the_branch_that_was_observed() {
        let (model, id) = model_with_a_slot();
        let mut facts = FactRegistry::new();
        facts.set(
            domux_core::facts::FactKey::workspace(&id, domux_core::facts::FACT_BRANCH),
            Some(branch_fact("feat/auth-cleanup")),
        );
        let text = drawn(&model, &facts, &ConfirmKind::DeleteWorkspace(id));
        assert!(text.contains("Delete workspace-1?"), "{text}");
        assert!(
            text.contains("/Users/pranav/projects/audrey-app/.domux/worktrees/workspace-1"),
            "{text}"
        );
        assert!(
            text.contains(
                "Removes the worktree, the local branch feat/auth-cleanup and closes 0 tabs."
            ),
            "the box's sentence does not repeat the path its identity line already shows: {text}"
        );
        assert!(
            text.contains("The remote branch and any pull request stay."),
            "{text}"
        );
        assert!(
            text.contains("y delete workspace    esc keep workspace"),
            "{text}"
        );
        // Inside the border there is nothing left of what was underneath. The margins still
        // hold `@`, which is the point: the box covers its own area and no more.
        let inside = text
            .lines()
            .find(|l| l.contains("Removes the worktree"))
            .map(|l| {
                let inner: String = l
                    .chars()
                    .skip_while(|c| *c != '\u{2502}')
                    .take_while(|c| *c != '@')
                    .collect();
                inner
            })
            .expect("the box drew its removes line");
        assert!(
            !inside.contains('@'),
            "the box covered what was under it: {inside}"
        );
        assert!(
            text.lines().all(|l| l.starts_with('@')),
            "and nothing outside it:\n{text}"
        );
    }

    /// And with no fact it names none. The pair is the point: with only the test above, a box
    /// that always printed the fact and one that fell back to the handle look the same.
    #[test]
    fn the_delete_box_names_no_branch_when_none_was_observed() {
        let (model, id) = model_with_a_slot();
        let text = drawn(
            &model,
            &FactRegistry::new(),
            &ConfirmKind::DeleteWorkspace(id),
        );
        assert!(
            text.contains("Removes the worktree, its local branch and closes 0 tabs."),
            "{text}"
        );
        assert!(!text.contains("the local branch workspace-1"), "{text}");
    }

    #[test]
    fn the_clear_box_says_what_goes_and_what_stays() {
        let (model, id) = model_with_a_slot();
        let text = drawn(
            &model,
            &FactRegistry::new(),
            &ConfirmKind::ClearWorkspace(id),
        );
        assert!(text.contains("Clear workspace-1?"), "{text}");
        assert!(
            text.contains("/Users/pranav/projects/audrey-app/.domux/worktrees/workspace-1"),
            "the identity line says which slot, in full: {text}"
        );
        assert!(
            text.contains(
                "Throws away every commit, change and untracked file in it and puts its \
                 branch back at its base."
            ),
            "{text}"
        );
        assert!(
            text.contains("The slot, its number, its name and the files git ignores stay."),
            "{text}"
        );
        // Principle 10's third answer, which for a clear is "nothing". Said rather than left
        // to be inferred: a dev server in the slot keeps running against a tree that changed.
        assert!(
            text.contains(
                "Nothing in its panes is stopped, so they keep running against the tree that \
                 changed."
            ),
            "{text}"
        );
        assert!(
            text.contains("y clear workspace    esc keep workspace"),
            "{text}"
        );
    }

    /// A branch name as long as the ones this program really produces.
    ///
    /// 63 columns, which puts the delete sentence at 120 against a budget of 112, so it is
    /// eight columns past the edge and has to break. The branch that prompted the measurement,
    /// `claude/PROJ-1482-rework-the-workspace-branch-provider`, is 53 and the box clipped
    /// above 55, so the shortened copy left two columns of headroom against what the author
    /// already types. A synthetic name would prove less than this one.
    const A_LONG_BRANCH: &str = "claude/PROJ-1482-rework-the-workspace-branch-provider-and-cache";

    /// Neither box loses a word at the width the harness calls a terminal, with a branch name
    /// long enough to have clipped before `wrapped` existed.
    ///
    /// Two assertions, because "no ellipsis" alone would pass on a box that dropped a line
    /// rather than breaking it: the sentences are rebuilt from the rows and matched whole, so
    /// what is checked is that every word arrived somewhere.
    ///
    /// 120 columns is the width every harness test in this milestone attaches at.
    #[test]
    fn neither_box_loses_a_word_at_a_hundred_and_twenty_columns() {
        let (model, id) = model_with_a_slot();
        let mut facts = FactRegistry::new();
        facts.set(
            domux_core::facts::FactKey::workspace(&id, domux_core::facts::FACT_BRANCH),
            Some(branch_fact(A_LONG_BRANCH)),
        );
        let delete = drawn_at(
            &model,
            &facts,
            &ConfirmKind::DeleteWorkspace(id.clone()),
            120,
        );
        assert!(
            !delete.contains('\u{2026}'),
            "the delete box lost words to its edge:\n{delete}"
        );
        assert!(
            reflowed(&delete).contains(&format!(
                "Removes the worktree, the local branch {A_LONG_BRANCH} and closes 0 tabs."
            )),
            "and the sentence is all there, across however many rows it took:\n{delete}"
        );

        let clear = drawn_at(&model, &facts, &ConfirmKind::ClearWorkspace(id), 120);
        assert!(
            !clear.contains('\u{2026}'),
            "the clear box lost words to its edge:\n{clear}"
        );
        assert!(
            reflowed(&clear).contains(
                "Throws away every commit, change and untracked file in it and puts its \
                 branch back at its base."
            ),
            "and so is this one:\n{clear}"
        );
    }

    /// The box's rows joined back into one string, so a sentence that was broken across two of
    /// them can be matched whole.
    fn reflowed(drawn: &str) -> String {
        let words: Vec<&str> = drawn
            .lines()
            .flat_map(|l| {
                l.trim_matches('@')
                    .trim_matches(|c: char| !c.is_alphanumeric() && c != ' ')
                    .split_whitespace()
            })
            .collect();
        words.join(" ")
    }

    /// The tab question draws no box here, which is a rule this file states and nothing tested.
    ///
    /// M1 asks it in the tab's own cell (`top_bar::draw`), so a box drawn here would put the
    /// same question on the screen twice. Pre-existing and cheap: the mutation that gives
    /// `CloseTab` a box of its own survived across twelve test binaries before this.
    #[test]
    fn the_tab_question_draws_nothing_because_the_tab_row_asks_it() {
        let (model, _id) = model_with_a_slot();
        let text = drawn(
            &model,
            &FactRegistry::new(),
            &ConfirmKind::CloseTab(TabId("t_0001".into())),
        );
        assert!(
            text.lines().all(|l| l.chars().all(|c| c == '@')),
            "nothing was drawn:\n{text}"
        );
    }

    /// A workspace the model no longer holds asks nothing rather than drawing a box about it,
    /// which is the same rule the project kind follows.
    #[test]
    fn a_workspace_the_model_does_not_hold_draws_no_question() {
        let (model, _id) = model_with_a_slot();
        let gone = WorkspaceId("w_gone".into());
        for kind in [
            ConfirmKind::DeleteWorkspace(gone.clone()),
            ConfirmKind::ClearWorkspace(gone.clone()),
        ] {
            let text = drawn(&model, &FactRegistry::new(), &kind);
            assert!(
                text.lines().all(|l| l.chars().all(|c| c == '@')),
                "nothing was drawn:\n{text}"
            );
        }
    }
}
