//! `project.*`: register a path, list what is registered, and let one go.

use super::{ok, Ctx};
use crate::core::CoreJob;
use domux_core::api::{
    Ack, ApiError, ProjectAddParams, ProjectAdded, ProjectInfo, ProjectRemoveParams,
};
use domux_core::ids::ProjectId;
use domux_core::model::{ConfirmKind, Focus, Overlay, Project, ProjectKind, RegionKind};
use serde_json::Value;
use std::path::Path;

/// What a project is: a repository, or a folder domux keeps workspaces beside. The word the
/// API answers with, in one place, so `project.list` and `project.add` cannot disagree.
pub fn kind_word(kind: &ProjectKind) -> &'static str {
    match kind {
        ProjectKind::Git { .. } => "git",
        ProjectKind::Folder => "folder",
    }
}

fn default_branch_of(kind: &ProjectKind) -> Option<String> {
    match kind {
        ProjectKind::Git { default_branch } => Some(default_branch.clone()),
        ProjectKind::Folder => None,
    }
}

/// Every registered project, in the order the model holds them.
pub fn list(ctx: &mut Ctx, _p: domux_core::api::NoParams) -> Result<Value, ApiError> {
    let projects: Vec<ProjectInfo> = ctx
        .model
        .projects
        .iter()
        .map(|p| ProjectInfo {
            id: p.id.clone(),
            name: p.name.clone(),
            root: p.root.clone(),
            kind: kind_word(&p.kind).to_string(),
            default_branch: default_branch_of(&p.kind),
            workspaces: p.workspaces.len(),
        })
        .collect();
    ok(projects)
}

/// Registers a path and adopts the worktrees it finds beside it.
///
/// Deciding what the path is means `git rev-parse`, `git symbolic-ref` and
/// `git worktree list`. Those are forks, and nothing that forks runs on the core task,
/// however fast it is. So the handler queues the reading and the reply travels with it
/// (decision record 0006); the model change happens in `Core::project_read`, back on the
/// core task, which is also where a path that is already registered is recognised, because
/// only the job knows the canonical path.
pub fn add(ctx: &mut Ctx, p: ProjectAddParams) -> Result<Value, ApiError> {
    ctx.jobs.push(CoreJob::ReadProject {
        path: p.path.clone(),
    });
    ctx.defer_reply = true;
    // Discarded: `defer_reply` means the job's answer is the caller's answer. A key press
    // has no caller waiting, and reads this as "the key did what it says", which is true -
    // the reading has started, and a failure reaches the hint row from `Core::answer`.
    ok(Ack { ok: true })
}

/// `1 workspace` or `2 workspaces`, so the question a reader is asked reads as a sentence.
fn workspaces_phrase(count: usize) -> String {
    match count {
        1 => "1 workspace".to_string(),
        n => format!("{n} workspaces"),
    }
}

/// What removing a project leaves alone. One sentence, written once: the API's refusal and
/// the confirmation overlay both say it, and a reader who sees one and then the other must
/// not be told two different things (principle 10, interface spec 7.3 and 12.8).
pub const KEEPS_THE_FOLDER: &str = "The folder and its worktrees stay on disk.";

/// What goes when a project is removed, as a noun phrase. The API lists it under the
/// question and the confirmation overlay puts it in a sentence, from this one spelling.
fn record_phrase(name: &str, workspaces: usize) -> String {
    format!(
        "the record of {name} and its {}",
        workspaces_phrase(workspaces)
    )
}

/// The words `project.remove` asks its question in, on either surface.
pub struct RemovalCopy {
    /// `Remove audrey-app?`, the confirmation's title.
    pub title: String,
    /// Where the project is, which is what tells two projects of the same name apart.
    pub identity: String,
    /// `Removes the record of audrey-app and its 2 workspaces.`
    pub removes: String,
}

/// The copy for one project, from the three things it depends on.
pub fn removal_copy(name: &str, workspaces: usize, root: &Path) -> RemovalCopy {
    RemovalCopy {
        title: format!("Remove {name}?"),
        identity: root.display().to_string(),
        removes: format!("Removes {}.", record_phrase(name, workspaces)),
    }
}

/// Removes a project's records. Nothing on disk is touched (interface spec 12.8).
///
/// It asks first, on whichever surface the caller is on: a key opens the confirmation
/// overlay, and a caller with a command line is refused with the question and told to add
/// `--yes`. Both refusals leave the model exactly as they found it.
pub fn remove(ctx: &mut Ctx, p: ProjectRemoveParams) -> Result<Value, ApiError> {
    let project = ctx.resolve_project_param(&p.project)?;
    // Unreachable: `resolve_project_param` has already answered with the id of a project
    // the model holds, and nothing runs between the two. Kept because the alternative is an
    // `expect` in a destructive handler, and written out rather than left silent so the next
    // reader can tell a considered choice from an oversight.
    let target = ctx
        .model
        .project(&project)
        .ok_or_else(|| ApiError::not_found(format!("no project called {}", p.project)))?;
    let (name, count, root) = (
        target.name.clone(),
        target.workspaces.len(),
        target.root.clone(),
    );
    if !p.yes {
        if ctx.from_key {
            let client = ctx.view()?;
            let Some(view) = ctx.model.client_mut(&client) else {
                return Err(ApiError::not_found(format!(
                    "client {client} is not attached"
                )));
            };
            view.push_overlay(Overlay::Confirm(ConfirmKind::RemoveProject(project)));
            view.focus = Focus::Region(RegionKind::Overlay);
            ctx.view_dirty = true;
            return ok(Ack { ok: true });
        }
        // `needs_confirmation` (Task 4) appends "Answer with --yes" and fills `data` with
        // the same content structured, so the CLI can lay it out as lists. There is no
        // `with_data` to chain onto `refused`.
        return Err(ApiError::needs_confirmation(
            format!(
                "Remove {name}? It has {}. {KEEPS_THE_FOLDER}",
                workspaces_phrase(count)
            ),
            vec![record_phrase(&name, count)],
            vec![format!(
                "the folder at {} and every worktree under it",
                root.display()
            )],
        ));
    }
    // Every pane under the project, before the records go. `remove_project` takes the tabs
    // and the panes with it without passing through `close_pane`, so nothing else would
    // ever kill these PTYs and the processes would outlive the project on screen.
    let doomed: Vec<domux_core::ids::PaneId> = ctx
        .model
        .workspaces_of(&project)
        .flat_map(|w| w.tabs.iter())
        .flat_map(|t| t.layout.pane_ids())
        .collect();
    ctx.events.extend(ctx.model.remove_project(&project)?);
    ctx.pending_kills.extend(doomed);
    let moved = reseat_stranded_clients(ctx.model);
    ctx.events.extend(moved);
    // Belt, and deliberately unpinned: `apply_side_effects` marks the view whenever it
    // killed anything, and a project always has at least one pane to kill, so no test can
    // tell this line from its absence. It stays because the day a project has no pane -
    // a workspace restored without a tab, a future kind that spawns nothing - the removal
    // must still redraw, and the cost of being wrong is a screen still showing a project
    // that is gone.
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Moves every client whose workspace has just gone onto a workspace that is still there.
///
/// A client points at a workspace and a tab. Removing the project a client is in leaves it
/// pointing at neither, and a frame drawn for such a client has no tab to draw and no pane
/// to send keys to (principle 2). With no project left there is nowhere to move it, and it
/// is left where it is: `render::draw_panes` draws no panes for a tab the model does not
/// hold, which is the honest picture of a domux with nothing in it.
///
/// Over the model rather than over a `Ctx`, because `workspace.delete` strands a client the
/// same way and answers from the core's `Deleted` arm, where there is no `Ctx`. One rule,
/// one implementation.
pub fn reseat_stranded_clients(
    model: &mut domux_core::model::Model,
) -> Vec<domux_core::api::Event> {
    let landing = model
        .projects
        .iter()
        .flat_map(|p| p.workspaces.iter())
        .flat_map(|w| w.tabs.first())
        .map(|t| t.id.clone())
        .next();
    let Some(tab) = landing else {
        return Vec::new();
    };
    let stranded: Vec<domux_core::ids::ClientId> = model
        .clients
        .iter()
        .filter(|view| model.workspace(&view.workspace).is_none())
        .map(|view| view.id.clone())
        .collect();
    let mut events = Vec::new();
    for client in stranded {
        match model.select_tab(&client, &tab) {
            Ok(more) => events.extend(more),
            Err(e) => tracing::error!("could not seat client {client} on tab {tab}: {}", e.message),
        }
    }
    events
}

/// What `project.add` answers with: the project as it now stands, and what this call
/// adopted. Built here rather than in the core so one place decides what a project looks
/// like on the wire.
pub fn added(project: &Project, adopted: Vec<String>) -> Result<Value, ApiError> {
    let main = project
        .workspaces
        .iter()
        .find(|w| w.handle == domux_core::model::WorkspaceHandle::Main)
        .ok_or_else(|| {
            ApiError::internal(format!("project {} has no main workspace", project.name))
        })?;
    ok(ProjectAdded {
        project: project.id.clone(),
        workspace: main.id.clone(),
        name: project.name.clone(),
        root: project.root.clone(),
        kind: kind_word(&project.kind).to_string(),
        adopted,
    })
}

impl Ctx<'_> {
    /// The project an `id` or a name names. The rule is `Model::resolve_project`'s; this is
    /// the handler's way in, beside `resolve_tab_param` and `resolve_pane_param`.
    pub fn resolve_project_param(&self, target: &str) -> Result<ProjectId, ApiError> {
        self.model.resolve_project(target)
    }
}
