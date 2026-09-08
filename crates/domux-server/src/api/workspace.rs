//! `workspace.*`: making a slot, and what follows from having made one.

use super::{ok, Ctx};
use crate::core::{slot_claim, CoreJob};
use domux_core::api::{Ack, ApiError, WorkspaceCreateParams};
use domux_core::ids::ProjectId;
use domux_core::model::ProjectKind;
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

impl Ctx<'_> {
    /// The project the calling client is looking at, for a call that named none.
    pub fn project_of_view(&self) -> Result<ProjectId, ApiError> {
        let client = self.view()?;
        let workspace = self
            .model
            .client(&client)
            .map(|view| view.workspace.clone())
            .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
        self.model
            .project_of_workspace(&workspace)
            .map(|p| p.id.clone())
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "client {client} is in a workspace no project holds; name the project"
                ))
            })
    }
}
