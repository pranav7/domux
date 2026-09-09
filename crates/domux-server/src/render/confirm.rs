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
    let area = overlay::centred_area(
        widest.saturating_add(CHROME),
        question.lines.len() as u16 + 2,
        buf,
    );
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
    for (i, line) in question.lines.iter().enumerate() {
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
            // is asked about by that branch, because that is the branch the delete removes;
            // a slot whose branch fact has not arrived is asked about without one
            // (principle 4). `api::workspace::delete` reads the same fact for the same reason.
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
                    Line::from(Span::styled(copy.removes, Style::default().fg(theme::TEXT))),
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
            let root = input
                .model
                .project_of_workspace(id)
                .map(|p| p.root.clone())?;
            let copy = crate::api::workspace::clear_copy(&w.display_name(), &root, &w.path);
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

    /// A project at `/p` holding one slot, checked out somewhere the handle does not name.
    ///
    /// The path and the root are literals and nothing here touches a filesystem: the box is
    /// composed on the core task, so it reads the model and the facts and never asks git.
    fn model_with_a_slot() -> (Model, WorkspaceId) {
        let mut model = Model::new(1);
        let id = WorkspaceId("w_1".into());
        model.projects.push(Project {
            id: ProjectId("p_1".into()),
            name: "audrey-app".into(),
            root: "/p".into(),
            kind: ProjectKind::Folder,
            workspaces: vec![Workspace {
                id: id.clone(),
                handle: WorkspaceHandle::Slot(1),
                name: None,
                path: "/p/.domux/worktrees/workspace-1".into(),
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
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let v = view(Overlay::Confirm(kind.clone()));
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
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 24));
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
        assert!(text.contains("/p/.domux/worktrees/workspace-1"), "{text}");
        assert!(
            text.contains(
                "Removes the worktree at .domux/worktrees/workspace-1 and the local branch \
                 feat/auth-cleanup and closes 0 tabs."
            ),
            "{text}"
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
            text.contains("and its local branch and closes 0 tabs."),
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
            text.contains("/p/.domux/worktrees/workspace-1"),
            "the identity line says which slot, in full: {text}"
        );
        assert!(
            text.contains(
                "Throws away every commit, change and untracked file in the worktree at \
                 .domux/worktrees/workspace-1 and puts its branch back at its base."
            ),
            "{text}"
        );
        assert!(
            text.contains("The slot, its number, its name and the files git ignores stay."),
            "{text}"
        );
        assert!(
            text.contains("y clear workspace    esc keep workspace"),
            "{text}"
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
