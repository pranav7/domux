//! `project.*`: register a path, list what is registered, and let one go.

use super::{ok, Ctx};
use crate::core::CoreJob;
use domux_core::api::{
    Ack, ApiError, ProjectAddParams, ProjectAdded, ProjectInfo, ProjectRemoveParams,
};
use domux_core::ids::{AgentId, ProjectId, WorkspaceId};
use domux_core::model::{ConfirmKind, Focus, Overlay, Project, ProjectKind, RegionKind};
use serde_json::Value;
use std::path::{Path, PathBuf};

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
    if p.all {
        return remove_all(ctx, p.yes);
    }
    let project = match p.project.as_deref() {
        Some(target) => ctx.resolve_project_param(target)?,
        // `X` in the Projects box (interface spec 7.3). The cursor is the only target a key
        // carries, and it is a target the reader can see, because the fill is on the row it
        // names. Anywhere else a removal that names nothing stays a mistake rather than
        // becoming a removal of whichever project the caller happened to be in.
        None => ctx.project_of_cursor()?,
    };
    // Unreachable: both arms above answer with the id of a project the model holds, and
    // nothing runs between the two. Kept because the alternative is an `expect` in a
    // destructive handler, and written out rather than left silent so the next reader can
    // tell a considered choice from an oversight.
    let target = ctx
        .model
        .project(&project)
        .ok_or_else(|| ApiError::not_found(format!("no project with id {project}")))?;
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
    // The caches of every record that is about to go, read before it goes because afterwards
    // nothing can name them. `remove_project` drops the records without passing through
    // `dismiss_agent`, so nothing else frees the working words they hold or the transcripts
    // they cached. This is the harsher of the two removal paths: a clear leaves its records
    // behind as exited, so a later dismiss reclaims what it missed, but a record removed here
    // is gone and a word lost with it is lost for the life of the server.
    let doomed_workspaces: Vec<WorkspaceId> = ctx
        .model
        .workspaces_of(&project)
        .map(|w| w.id.clone())
        .collect();
    let doomed_records: Vec<(AgentId, Option<PathBuf>)> = doomed_workspaces
        .iter()
        .flat_map(|ws| ctx.model.agents_in_workspace(ws))
        .map(|a| (a.id.clone(), a.transcript_path.clone()))
        .collect();
    ctx.events.extend(ctx.model.remove_project(&project)?);
    for (id, transcript) in &doomed_records {
        ctx.agents.forget_record(id, transcript.as_deref());
    }
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

/// The question `--all` asks, in one place so the refusal and any later surface agree.
fn removes_everything(names: &[String]) -> String {
    format!(
        "Remove every project? There {} {}: {}. {KEEPS_THE_FOLDER}",
        if names.len() == 1 { "is" } else { "are" },
        projects_phrase(names.len()),
        names.join(", ")
    )
}

/// `1 project` or `3 projects`.
fn projects_phrase(count: usize) -> String {
    match count {
        1 => "1 project".to_string(),
        n => format!("{n} projects"),
    }
}

/// Lets every project go at once: the way back to an empty domux without deleting the state
/// file by hand.
///
/// It removes records and never files, exactly as removing one project does. What is left is
/// a server with no workspace, which is a state the screen already has words for: the
/// Projects box says "No projects yet" and names the command that ends it, and `attach`
/// offers to register the directory it was typed in. So this does not seed a replacement
/// project on the way out; inventing one would answer a different question from the one that
/// was asked.
///
/// There is no key for it and there is not going to be one. A key that removed every record
/// on the screen is not a key anybody should be one keystroke away from; the command line
/// asks first and takes `--yes`.
fn remove_all(ctx: &mut Ctx, yes: bool) -> Result<Value, ApiError> {
    let names: Vec<String> = ctx.model.projects.iter().map(|p| p.name.clone()).collect();
    // Nothing to remove is not a failure: the caller asked for no projects and there are
    // none. It answers without asking, because a question about an empty list has no answer
    // worth giving.
    if names.is_empty() {
        return ok(Ack { ok: true });
    }
    if !yes {
        return Err(ApiError::needs_confirmation(
            removes_everything(&names),
            names
                .iter()
                .map(|name| format!("the record of {name}"))
                .collect(),
            ctx.model
                .projects
                .iter()
                .map(|p| {
                    format!(
                        "the folder at {} and every worktree under it",
                        p.root.display()
                    )
                })
                .collect(),
        ));
    }
    let projects: Vec<ProjectId> = ctx.model.projects.iter().map(|p| p.id.clone()).collect();
    for project in projects {
        // Every pane under the project, before the records go, for the reason the single
        // removal gives: `remove_project` takes the tabs and the panes with it without
        // passing through `close_pane`, so nothing else would ever kill these PTYs.
        let doomed: Vec<domux_core::ids::PaneId> = ctx
            .model
            .workspaces_of(&project)
            .flat_map(|w| w.tabs.iter())
            .flat_map(|t| t.layout.pane_ids())
            .collect();
        ctx.events.extend(ctx.model.remove_project(&project)?);
        ctx.pending_kills.extend(doomed);
    }
    // Nothing to reseat a client onto once the last project is gone, and `reseat_stranded_clients`
    // says so by leaving every client where it is. Called anyway, because it is the one rule
    // about a stranded client and this path must not grow a second.
    let moved = reseat_stranded_clients(ctx.model);
    ctx.events.extend(moved);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Leaves no client pointing at a workspace that is gone: the one it is in, and the one its
/// cursor rests on.
///
/// A client points at a workspace and a tab. Removing the project a client is in leaves it
/// pointing at neither, and a frame drawn for such a client has no tab to draw and no pane
/// to send keys to (principle 2). With no project left there is nowhere to move it, and it
/// is left where it is: `render::draw_panes` draws no panes for a tab the model does not
/// hold, which is the honest picture of a domux with nothing in it.
///
/// The cursor is the same question asked of the Projects box. A cursor on a row that has
/// just gone draws no fill at all, so `X` on the last row of a project would answer the
/// question and then leave the box with nothing marked in it. Clearing it puts the fill back
/// on the workspace the client is in, which is what the box shows when nobody has moved the
/// cursor (domain model, section 3.3).
///
/// Over the model rather than over a `Ctx`, because `workspace.delete` strands a client the
/// same way and answers from the core's `Deleted` arm, where there is no `Ctx`. One rule,
/// one implementation.
pub fn reseat_stranded_clients(
    model: &mut domux_core::model::Model,
) -> Vec<domux_core::api::Event> {
    // The cursors first, and before the early return below: with the last project gone every
    // cursor is stale, and that is the case where leaving one behind lasts longest.
    let stale: Vec<domux_core::ids::ClientId> = model
        .clients
        .iter()
        .filter(|view| {
            view.projects_cursor
                .as_ref()
                .is_some_and(|w| model.workspace(w).is_none())
        })
        .map(|view| view.id.clone())
        .collect();
    for client in stale {
        if let Some(view) = model.client_mut(&client) {
            view.projects_cursor = None;
        }
    }
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
