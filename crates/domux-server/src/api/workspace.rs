//! `workspace.*`: making a slot, and what follows from having made one.

use super::{ok, Ctx};
use crate::core::{slot_claim, CoreJob};
use domux_core::api::{
    Ack, ApiError, Event, WorkspaceCreateParams, WorkspaceFocusParams, WorkspaceInfo,
    WorkspaceListParams, WorkspaceRenameParams, WorkspaceTargetParams,
};
use domux_core::facts::{FactKey, FACT_BRANCH, FACT_PR};
use domux_core::ids::{ProjectId, WorkspaceId};
use domux_core::model::{Focus, Overlay, ProjectKind, RegionKind};
use serde_json::Value;

/// Makes the next slot of a project: a worktree at the lowest free number, on a fresh branch
/// from the base, with the project's `worktree.conf` applied to it.
///
/// `git fetch` and `git worktree add` take seconds and nothing that shells out runs on the
/// core task, so this handler chooses the number and queues the work, and the caller's answer
/// travels with the job (decision record 0006). Everything it can refuse, it refuses here,
/// before the job exists: a refusal that arrived after `git worktree add` had run would carry
/// the same error code as this one and leave a directory and a branch behind it.
///
/// The number is the one thing this handler chooses rather than reads, so it claims it.
/// `lowest_free_slot` answers from the model, the model learns the slot when the job
/// finishes, and two creates arriving in that gap would otherwise both pick the lowest free
/// number and both try to build it: one would win and the other would fail inside git, in
/// git's words about a directory rather than domux's about a slot. With the claim they pick
/// two different numbers and both work, which is what the caller asked for either way.
pub fn create(ctx: &mut Ctx, p: WorkspaceCreateParams) -> Result<Value, ApiError> {
    let project = match &p.project {
        Some(name) => ctx.resolve_project_param(name)?,
        None => ctx.project_of_view()?,
    };
    // Unreachable: `resolve_project_param` and `project_of_view` both answer with the id of a
    // project the model holds, and nothing runs between the two. Written out rather than
    // left as an `expect`, and rather than left silent, so the next reader can tell a
    // considered choice from an oversight.
    let target = ctx
        .model
        .project(&project)
        .ok_or_else(|| ApiError::not_found(format!("no project with id {project}")))?;
    let ProjectKind::Git { .. } = &target.kind else {
        return Err(ApiError::refused(format!(
            "a plain folder has only main; {} is not a git repository",
            target.root.display()
        )));
    };
    let root = target.root.clone();
    let claims = ctx.claims;
    let slot = ctx
        .model
        .lowest_free_slot(&project, |n| claims.contains(&slot_claim(&project, n)))?;
    ctx.jobs.push(CoreJob::CreateWorkspace {
        root: root.clone(),
        slot,
        path: crate::git::slot_path(&root, slot),
        branch: crate::git::slot_branch(slot),
        // What was asked for, not what it resolves to: `git::base_ref` reads `origin/HEAD`
        // when nothing was asked for, and that is a fork, so it belongs in the job.
        base: p
            .base
            .clone()
            .or_else(|| ctx.config.config.worktrees.base.clone()),
        project,
    });
    ctx.defer_reply = true;
    // Discarded: `defer_reply` means the job's answer is the caller's answer. A key press has
    // no caller waiting and reads this as "the key did what it says", which is true - the
    // work has started, and a failure reaches the hint row from `Core::answer`.
    ok(Ack { ok: true })
}

/// One workspace as the API reports it: what domux decided about it, plus what it observed.
///
/// `branch`, `pr` and `pr_state` are read from the facts and each is absent when its fact is.
/// A workspace with no pull request answers `null`, never an empty string: a caller that
/// cannot tell an observation from a blank has been given a guess (principle 4).
fn info(ctx: &Ctx, id: &WorkspaceId) -> Result<WorkspaceInfo, ApiError> {
    let project = ctx
        .model
        .project_of_workspace(id)
        .map(|p| p.id.clone())
        .ok_or_else(|| ApiError::not_found(format!("no workspace with id {id}")))?;
    // Unreachable: `project_of_workspace` found the project by walking to this workspace, so
    // the model holds it. Written out rather than left as an `expect` message the next
    // reader has to reason about.
    let w = ctx
        .model
        .workspace(id)
        .ok_or_else(|| ApiError::not_found(format!("no workspace with id {id}")))?;
    let pr = ctx.facts.get(&FactKey::workspace(id, FACT_PR));
    Ok(WorkspaceInfo {
        id: w.id.clone(),
        project,
        handle: w.handle.to_string(),
        name: w.name.clone(),
        path: w.path.clone(),
        branch: ctx
            .facts
            .get(&FactKey::workspace(id, FACT_BRANCH))
            .map(|f| f.text.clone()),
        pr: pr.map(|f| f.text.clone()),
        pr_state: pr.and_then(|f| f.state.as_ref().map(|s| s.to_string())),
        tabs: w.tabs.len(),
    })
}

/// Every workspace of a project, or of every project when none is named.
///
/// Listing everything is what "no project" has to mean here, because it is the answer the
/// rest of the code already promises: `Model::resolve_workspace_with` searches every project
/// and tells a caller whose target matched nothing to run `workspace list` to see them. A
/// list scoped to one project could not show what that search looked at.
///
/// The order is the model's: projects as they were added, and inside a project `main` first
/// and then the slots in number order, which `Model::add_slot` keeps.
pub fn list(ctx: &mut Ctx, p: WorkspaceListParams) -> Result<Value, ApiError> {
    let ids: Vec<WorkspaceId> = match &p.project {
        Some(name) => {
            let project = ctx.resolve_project_param(name)?;
            ctx.model
                .workspaces_of(&project)
                .map(|w| w.id.clone())
                .collect()
        }
        None => ctx
            .model
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .map(|w| w.id.clone())
            .collect(),
    };
    let rows: Result<Vec<WorkspaceInfo>, ApiError> = ids.iter().map(|id| info(ctx, id)).collect();
    ok(rows?)
}

/// This client shows `workspace` on its last focused tab, and the keys go to that tab's
/// focused pane (interface spec 5.4 and 12.26).
///
/// A workspace with no tabs gets one with a shell in its own path, so switching always lands
/// somewhere you can type. **That branch cannot be reached today**: `Core::apply_side_effects`
/// runs `ensure_every_workspace_has_a_tab` at the end of every dispatch and `Core::start` runs
/// it once more, so by the time a handler runs no workspace in the model is without a tab, and
/// no test in the suite can enter the branch. It is kept because this handler should be right
/// on its own terms rather than on an invariant two layers away, and it is written down here
/// because untested-and-silent and deliberately-unspecified look the same in a diff. A mutant
/// that removes it survives, and that is recorded rather than argued away.
///
/// The switcher closes because switching is what it was open for. The sidebar does not: it
/// is not an overlay, the fill moves to the row that is now the current one, and the keys go
/// to the pane (interface spec 12.26).
pub fn focus(ctx: &mut Ctx, p: WorkspaceFocusParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    // Before anything is read or made. A call naming a client that is not attached must
    // change nothing rather than half of it: without this guard it would still move
    // `last_workspace` and give the target a tab and a shell on behalf of a client that does
    // not exist, and the error code would look the same either way.
    if ctx.model.client(&client).is_none() {
        return Err(ApiError::not_found(format!(
            "client {client} is not attached"
        )));
    }
    let target = ctx.resolve_workspace_param(Some(&p.workspace))?;
    // Unreachable: `resolve_workspace_param` answers with the id of a workspace the model
    // holds, and nothing runs between the two. Written out rather than left as an `expect`,
    // and rather than left silent, so the next reader can tell a considered choice from an
    // oversight.
    let w = ctx
        .model
        .workspace(&target)
        .ok_or_else(|| ApiError::not_found(format!("no workspace called {}", p.workspace)))?;
    let path = w.path.clone();
    let mut tab = w
        .last_tab
        .clone()
        .or_else(|| w.tabs.first().map(|t| t.id.clone()));
    if tab.is_none() {
        let (new_tab, pane, events) = ctx.model.create_tab(&target, path)?;
        ctx.events.extend(events);
        // M1's `Ctx` has no `spawn`. A handler is pure over the model and the runtime maps;
        // spawning a process is the core's, so it is recorded and the core does it.
        ctx.pending_spawns.push(pane);
        tab = Some(new_tab);
    }
    let tab = tab.expect("the branch above gives a workspace with no tabs one");
    ctx.model.last_workspace = Some(target.clone());
    // Nothing writes `last_tab` back here, and that is a decision rather than an omission.
    // The model already maintains it everywhere a tab is made, chosen or closed:
    // `create_tab` sets it, `select_tab` sets it, and `close_tab` moves it to the tab that
    // took the closed one's place. So a workspace with tabs always has `last_tab` set, the
    // `or_else` fallback above can only fire for a workspace with none, and writing `tab`
    // back would be the identity in both branches. A mutant that deleted such a line would
    // survive, and an equivalent line that cannot fail is worse than no line: it reads as a
    // second owner of a field that has one.
    let pane = ctx.model.tab(&tab).map(|t| t.focused.clone());
    let in_switcher = ctx
        .model
        .client(&client)
        .map(|view| view.overlay == Some(Overlay::Switcher))
        .unwrap_or(false);
    if let Some(view) = ctx.model.client_mut(&client) {
        view.workspace = target.clone();
        view.tab = tab.clone();
        // The fill follows: the current row is the row of the workspace this client is in
        // (domain model 3.3), and after a switch that is the target.
        view.projects_cursor = Some(target.clone());
        if in_switcher {
            // Only the switcher, and only the top level of it. The sidebar is not an
            // overlay, so nothing here touches it and it stays open (interface spec 12.26).
            view.pop_overlay();
        }
        // Never a frame with the keys in a region nothing on the screen marks (principle 2).
        // The pane takes them, unless closing the switcher uncovered the overlay it was
        // opened over (interface spec 12.7), which is then what the reader is looking at.
        view.focus = match (&view.overlay, pane) {
            (Some(_), _) => Focus::Region(RegionKind::Overlay),
            (None, Some(pane)) => Focus::Pane(pane),
            (None, None) => view.focus.clone(),
        };
    }
    ctx.events.push(Event::WorkspaceSwitched {
        client,
        workspace: target.clone(),
        tab,
    });
    ctx.view_dirty = true;
    ok(info(ctx, &target)?)
}

/// Gives a workspace a name, which then stands in for its handle on every row and resolves
/// it everywhere a target is taken (architecture spec 2).
///
/// A blank name clears it and the handle comes back, the rule `Model::rename_workspace`
/// holds and the one `tab.rename` follows. **No name at all is a different request**: it
/// means "ask me for one here", and the name box that asks is not built yet. So this refuses
/// rather than reading no name as a blank one, which would clear the name of the workspace
/// the reader was about to name.
pub fn rename(ctx: &mut Ctx, p: WorkspaceRenameParams) -> Result<Value, ApiError> {
    let target = ctx.resolve_workspace_param(p.workspace.as_deref())?;
    let Some(name) = p.name else {
        return Err(ApiError::unavailable(
            "give a name: naming a workspace on the screen arrives with the name box",
        ));
    };
    let events = ctx.model.rename_workspace(&target, Some(name))?;
    ctx.events.extend(events);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Takes the name off, so the handle comes back.
pub fn clear_name(ctx: &mut Ctx, p: WorkspaceTargetParams) -> Result<Value, ApiError> {
    let target = ctx.resolve_workspace_param(p.workspace.as_deref())?;
    let events = ctx.model.rename_workspace(&target, None)?;
    ctx.events.extend(events);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// The stub the roadmap's 5.7 table names. M3 replaces it with the real resume, which puts
/// an agent back in the pane it was working in.
///
/// It refuses without looking at its target on purpose: resolving a workspace first would
/// answer `not_found` for a bad target and `unavailable` for a good one, which reads as a
/// method that half works. There is nothing here to work.
pub fn resume(_ctx: &mut Ctx, _p: WorkspaceTargetParams) -> Result<Value, ApiError> {
    Err(ApiError::unavailable("resume arrives with agents in M3"))
}

impl Ctx<'_> {
    /// The workspace a `workspace` param names, or the calling view's workspace.
    ///
    /// The branch pass is why this lives on `Ctx` rather than on the model: `domux-core` has
    /// no fact registry, so `resolve_workspace_with` takes the branch of a workspace as a
    /// lookup and the server hands it one that reads the facts. That is what lets a call
    /// from a shell on branch `feat/auth-cleanup` name its workspace without an id.
    pub fn resolve_workspace_param(
        &self,
        workspace: Option<&str>,
    ) -> Result<WorkspaceId, ApiError> {
        match workspace {
            Some(target) => self.model.resolve_workspace_with(target, &|id| {
                self.facts
                    .get(&FactKey::workspace(id, FACT_BRANCH))
                    .map(|f| f.text.clone())
            }),
            None => self.workspace_of_view(),
        }
    }

    /// The workspace the calling client is in, for a call that named none.
    pub fn workspace_of_view(&self) -> Result<WorkspaceId, ApiError> {
        let client = self.view()?;
        self.model
            .client(&client)
            .map(|view| view.workspace.clone())
            .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))
    }

    /// The project the calling client is looking at, for a call that named none.
    pub fn project_of_view(&self) -> Result<ProjectId, ApiError> {
        let workspace = self.workspace_of_view()?;
        self.model
            .project_of_workspace(&workspace)
            .map(|p| p.id.clone())
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "workspace {workspace} is in no project; name the project"
                ))
            })
    }
}
