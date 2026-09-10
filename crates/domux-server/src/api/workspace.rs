//! `workspace.*`: making a slot, and what follows from having made one.

use super::{ok, Ctx};
use crate::core::{slot_claim, CoreJob};
use domux_core::api::{
    Ack, AgentResumeResult, ApiError, Event, WorkspaceClearParams, WorkspaceCreateParams,
    WorkspaceDeleteParams, WorkspaceFocusParams, WorkspaceInfo, WorkspaceListParams,
    WorkspaceRenameParams, WorkspaceResumeResult, WorkspaceTargetParams,
};
use domux_core::facts::{FactKey, FACT_BRANCH, FACT_PR};
use domux_core::ids::{PaneId, ProjectId, WorkspaceId};
use domux_core::model::{
    ConfirmKind, Focus, Overlay, ProjectKind, RegionKind, TextInput, WorkspaceHandle,
    MAIN_CANNOT_BE_DELETED,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
/// **It does not make a tab.** Every workspace has one because
/// `Core::ensure_every_workspace_has_a_tab` gives it one: `Core::new` runs that once and
/// `apply_side_effects` runs it as its last statement, which is after every dispatch. So that
/// invariant has one owner, and a switch that quietly repaired a workspace without a tab would
/// be a second - a repair no input could reach, and therefore no test could check. This reports
/// the invariant broken instead, naming the workspace and what should have run.
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
    // `last_tab` answers both questions at once - which tab, and whether there is one -
    // because the model keeps the two together: `create_tab` sets it, `select_tab` sets it,
    // and `close_tab` moves it to the tab that took the closed one's place, leaving it `None`
    // only when the workspace has no tabs left. So there is no fallback to `tabs.first()`
    // here: a workspace with tabs always has `last_tab` set, and a fallback that no input can
    // reach would be a second answer to a question the model already answers once.
    let tab = w.last_tab.clone().ok_or_else(|| {
        // The one owner of "every workspace has a tab" is
        // `Core::ensure_every_workspace_has_a_tab`. Reaching this means it did not run, so
        // the answer names the workspace and the thing that should have run, which is what a
        // reader can act on. Making a tab here instead would hide that and give the invariant
        // a second owner.
        ApiError::internal(format!(
            "workspace {target} has no tab; ensure_every_workspace_has_a_tab did not run for it"
        ))
    })?;
    ctx.model.last_workspace = Some(target.clone());
    // Nothing writes `last_tab` back, for the reason above: `tab` came from it, so the write
    // would be the identity, and an equivalent line that cannot fail is worse than no line.
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
/// means "ask me for one here", so it opens the name box rather than reading no name as a
/// blank one, which would clear the name of the workspace the reader was about to name.
pub fn rename(ctx: &mut Ctx, p: WorkspaceRenameParams) -> Result<Value, ApiError> {
    let target = ctx.resolve_workspace_param(p.workspace.as_deref())?;
    let Some(name) = p.name else {
        return open_name_box(ctx, target);
    };
    // A name that reads as a handle can never find the workspace it was given to. The handle
    // pass of `Model::resolve_workspace_with` runs before the name pass and returns as soon as
    // it has one hit, so `workspace-2` would answer with the workspace whose handle that is,
    // and the row would draw the same words in two places. Refusing here is the only place
    // that can hold the line: `workspace.clear` and `workspace.delete` take their target
    // through that resolver, so a name nobody can resolve is a worktree removed by mistake.
    //
    // The grammar, not a check against the handles the model holds today: a name that is free
    // now would be shadowed the moment someone made that slot, and nothing would look again.
    // A name another workspace already has is a different matter and is allowed - two projects
    // can each have an `auth` - because the resolver answers that with an ambiguity naming
    // both, rather than silently picking one.
    if WorkspaceHandle::reads_as_handle(&name) {
        return Err(ApiError::invalid_params(format!(
            "{} is a handle, and a handle answers first, so nothing could find this workspace by that name; pick another",
            name.trim()
        )));
    }
    let events = ctx.model.rename_workspace(&target, Some(name))?;
    ctx.events.extend(events);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Opens the name box on `target` in the calling view (interface spec 7.1), with the name the
/// workspace has already in it so that fixing a typo does not mean retyping the whole name.
///
/// `leader N`, `n` on a row and `workspace name` with no argument all arrive here, because all
/// three are `workspace.rename` with no name. Which workspace they name is
/// `Ctx::workspace_of_view`'s answer, which is the row under the cursor while the keys are in
/// a Projects box and the client's own workspace otherwise.
fn open_name_box(ctx: &mut Ctx, target: WorkspaceId) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let current = ctx
        .model
        .workspace(&target)
        .and_then(|w| w.name.clone())
        .unwrap_or_default();
    // Before anything is written: a call on behalf of a client that is not attached must
    // leave the model as it found it, and there is no screen to put a box on anyway.
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    view.input = TextInput::new(current);
    // The box's hint row is its own, and the result of whatever the reader did before they
    // opened it is not an answer to anything in it (interface spec 12.12). Cleared here
    // rather than on the first key in the box, so the row that opens says what the two keys
    // do rather than repeating the last thing that happened.
    view.pill = None;
    // Replaced, not stacked, when a name box is already open. `push_overlay` keeps one level
    // underneath, so a second open would put this box over the first and leave whatever the
    // first was opened over - the switcher, when `n` opened it - with nothing drawing it and
    // nothing closing it. Only a caller can reach this: a key cannot, because the open box
    // takes every key.
    if matches!(view.overlay, Some(Overlay::NameWorkspace(_))) {
        view.overlay = Some(Overlay::NameWorkspace(target));
    } else {
        view.push_overlay(Overlay::NameWorkspace(target));
    }
    view.focus = Focus::Region(RegionKind::Overlay);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Takes the name off, so the handle comes back. `leader n` presses it, with no prompt and no
/// question: the row redrawing with its handle is the answer (interface spec 12.9).
pub fn clear_name(ctx: &mut Ctx, p: WorkspaceTargetParams) -> Result<Value, ApiError> {
    let target = ctx.resolve_workspace_param(p.workspace.as_deref())?;
    let events = ctx.model.rename_workspace(&target, None)?;
    ctx.events.extend(events);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Why `main` is refused by both destructive operations: it is the project's own checkout,
/// so clearing it would reset the author's own work and deleting it would take the
/// repository (architecture spec: "path = project root; can't be cleared or deleted").
///
/// The delete wording is `domux_core::model::MAIN_CANNOT_BE_DELETED` and not a copy of it:
/// `Model::remove_workspace` refuses the same thing, and one refusal spelled in two places
/// is one refusal that can come to say two things.
const CLEAR_REFUSES_MAIN: &str =
    "main is the project's checkout and cannot be cleared; clear a workspace-N slot instead";

/// `1 tab` or `2 tabs`, so the question a reader is asked reads as a sentence.
fn tabs_phrase(count: usize) -> String {
    match count {
        1 => "1 tab".to_string(),
        n => format!("{n} tabs"),
    }
}

/// The branch a delete would remove, as a noun phrase, from the one thing that can know it.
///
/// `None` is the branch provider not having answered yet. It says "its local branch" rather
/// than naming the handle, because the handle is a record and the branch is a fact: a slot
/// checked out on `feat/auth-cleanup` still has the handle `workspace-1`, and a question
/// that named the handle would promise to delete a branch this call is not going to touch
/// (principle 4).
fn branch_phrase(branch: Option<&str>) -> String {
    match branch {
        Some(b) => format!("the local branch {b}"),
        None => "its local branch".to_string(),
    }
}

/// Where a slot sits inside its project, which is what the question calls it. The absolute
/// path is what the overlay's identity line shows; a caller that named the workspace already
/// knows the project, so the sentence uses the short form.
fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The words `workspace.delete` asks its question in, on either surface. One builder, so the
/// confirmation on the screen and the one a shell prints cannot drift apart (principle 10,
/// interface spec 7.3 and 12.22).
pub struct DeletionCopy {
    /// `Delete auth cleanup?`, the confirmation's title and the question's first sentence.
    pub title: String,
    /// Where the slot is, in full. The one line that tells two `workspace-1`s apart, which
    /// is why it is the absolute path and not the short form the sentence uses.
    pub identity: String,
    /// `Removes the worktree at .domux/worktrees/workspace-1 and the local branch
    /// workspace-1 and closes 1 tab.` For a caller that has nothing above the sentence
    /// saying where the slot is, which is every caller with a command line.
    pub removes: String,
    /// The same sentence without the path, for a surface that draws `identity` over it.
    ///
    /// Not a second spelling: both are built in `deletion_copy` from the same `worktree`,
    /// `branch` and `tabs_phrase`, so they cannot come to disagree about what goes. What
    /// differs is the one thing that should: the overlay says where the slot is on its own
    /// line and the sentence would only repeat it, and at 120 columns the repetition is what
    /// pushes the branch off the end of the box. `confirm::draw` clips rather than wraps, so
    /// a line that does not fit loses its tail.
    pub removes_without_the_path: String,
    /// What domux does not reach. Nothing here talks to a remote.
    pub keeps: &'static str,
    /// The same content as lists, for `ApiError::needs_confirmation`.
    pub removes_list: Vec<String>,
    pub keeps_list: Vec<String>,
}

impl DeletionCopy {
    /// The whole question in one sentence, for a caller that shows messages rather than
    /// laying out lists.
    pub fn question(&self) -> String {
        format!("{} {} {}", self.title, self.removes, self.keeps)
    }
}

pub const DELETE_KEEPS: &str = "The remote branch and any pull request stay.";

/// The copy for one delete, from the five things it depends on.
pub fn deletion_copy(
    name: &str,
    root: &Path,
    path: &Path,
    branch: Option<&str>,
    tabs: usize,
) -> DeletionCopy {
    let relative = relative_to(root, path);
    let worktree = format!("the worktree at {relative}");
    let branch = branch_phrase(branch);
    DeletionCopy {
        title: format!("Delete {name}?"),
        identity: path.display().to_string(),
        removes: format!(
            "Removes {worktree} and {branch} and closes {}.",
            tabs_phrase(tabs)
        ),
        removes_without_the_path: format!(
            "Removes the worktree, {branch} and closes {}.",
            tabs_phrase(tabs)
        ),
        keeps: DELETE_KEEPS,
        removes_list: vec![worktree, branch],
        keeps_list: vec!["the remote branch and any pull request".to_string()],
    }
}

/// The words `workspace.clear` asks its question in. Only the overlay asks it: a caller with
/// a command line is refused by the job, which is the only thing that can know whether there
/// is anything in the slot to lose.
pub struct ClearCopy {
    pub title: String,
    pub identity: String,
    pub removes: String,
    pub keeps: &'static str,
    /// What a clear does not stop. Principle 10 asks for what is removed, **stopped** and
    /// preserved, and a clear is the case where the third answer is "nothing": the shell in
    /// pane 2 is the author's and killing it is not part of putting a branch back.
    ///
    /// It is said rather than left to be inferred, because the surprise is the quiet one. A
    /// dev server running in the slot keeps running, against a tree that changed underneath
    /// it, and a reader who was told only what goes has no reason to expect that.
    pub stops: &'static str,
}

/// The base is not named. Resolving it reads `origin/HEAD`, which is a git call and does not
/// belong on the core task, and a question that guessed `origin/main` would be a fact nobody
/// observed (principle 4). The files git ignores are named because that is where a project's
/// `worktree.conf` setup puts `.env`, and `git::clean` is `-fd` for exactly that reason.
pub const CLEAR_KEEPS: &str = "The slot, its number, its name and the files git ignores stay.";

/// Nothing in the slot's panes is killed, restarted or told anything. The clause exists
/// because that is not what a reader expects of a command that empties the directory those
/// programs are running in.
pub const CLEAR_STOPS: &str =
    "Nothing in its panes is stopped, so they keep running against the tree that changed.";

/// The path is not repeated: the overlay is this copy's only reader and its identity line
/// shows the worktree in full, one line above.
pub fn clear_copy(name: &str, path: &Path) -> ClearCopy {
    ClearCopy {
        title: format!("Clear {name}?"),
        identity: path.display().to_string(),
        removes:
            "Throws away every commit, change and untracked file in it and puts its branch back at its base."
                .to_string(),
        keeps: CLEAR_KEEPS,
        stops: CLEAR_STOPS,
    }
}

/// Everything both destructive handlers read off the model before they queue anything.
///
/// One reader, because the two must agree about which workspace, which branch and which
/// project they are acting on. `verb` is the word the `main` refusal uses, so the two
/// refusals differ only where they should.
fn target_of(
    ctx: &Ctx,
    workspace: Option<&str>,
    refuses_main: &str,
) -> Result<(WorkspaceId, Doomed), ApiError> {
    let target = ctx.resolve_workspace_param(workspace)?;
    // Unreachable: `resolve_workspace_param` answers with the id of a workspace the model
    // holds, and nothing runs between the two. Written out rather than left as an `expect`
    // in a handler that removes worktrees, and rather than left silent, so the next reader
    // can tell a considered choice from an oversight.
    let w = ctx
        .model
        .workspace(&target)
        .ok_or_else(|| ApiError::not_found(format!("no workspace with id {target}")))?;
    if w.handle == WorkspaceHandle::Main {
        return Err(ApiError::refused(refuses_main));
    }
    let doomed = Doomed {
        name: w.display_name(),
        path: w.path.clone(),
        tabs: w.tabs.len(),
        // What the branch provider observed, not what the handle is called. `None` means it
        // has not answered; the job reads the branch itself before it removes anything.
        branch: ctx
            .facts
            .get(&FactKey::workspace(&target, FACT_BRANCH))
            .map(|f| f.text.clone()),
        root: ctx
            .model
            .project_of_workspace(&target)
            .map(|p| p.root.clone())
            .ok_or_else(|| ApiError::not_found(format!("workspace {target} is in no project")))?,
        base: ctx.config.config.worktrees.base.clone(),
    };
    Ok((target, doomed))
}

/// What a destructive handler read, in the shape both the copy and the job need.
struct Doomed {
    name: String,
    root: PathBuf,
    path: PathBuf,
    tabs: usize,
    branch: Option<String>,
    base: Option<String>,
}

/// Puts a slot back where it started: its branch at the base, nothing uncommitted, nothing
/// untracked. The slot, its number, its name and its tabs stay (architecture spec, M2 row).
///
/// **It does not ask when there is nothing to lose.** Whether there is anything to lose is
/// `git::is_dirty`, which shells out, so the question cannot be asked here: the job checks
/// and refuses. That is the difference from `delete`, which always asks - a delete takes the
/// slot itself, so even a pristine one is a change the caller may not have meant (interface
/// spec 12.22 says so for delete and says nothing about clear).
///
/// A key press has no `--yes` to add, so from a key it asks on the screen first, every time.
/// The alternative would be a red pill telling a reader with no command line to use a flag.
pub fn clear(ctx: &mut Ctx, p: WorkspaceClearParams) -> Result<Value, ApiError> {
    let (target, doomed) = target_of(ctx, p.workspace.as_deref(), CLEAR_REFUSES_MAIN)?;
    if ctx.from_key && !p.yes {
        return ask(ctx, ConfirmKind::ClearWorkspace(target));
    }
    ctx.jobs.push(CoreJob::ClearWorkspace {
        workspace: target,
        name: doomed.name,
        root: doomed.root,
        path: doomed.path,
        base: doomed.base,
        yes: p.yes,
    });
    ctx.defer_reply = true;
    // Discarded: `defer_reply` means the job's answer is the caller's answer. A key press has
    // no caller waiting and reads this as "the key did what it says", which is true - the
    // work has started, and a failure reaches the hint row from `Core::answer`.
    ok(Ack { ok: true })
}

/// Removes a slot: the worktree, its local branch and its record, with every tab and pane
/// under it (interface spec 7.3, 12.22).
///
/// It asks first on whichever surface the caller is on, and it asks every time: a key opens
/// the confirmation overlay, and a caller with a command line is refused with the question
/// and told to add `--yes`. Both refusals leave the model and the disk exactly as they found
/// them.
///
/// **The question names the branch fact; the job reads the branch itself.** Nothing stops the
/// author checking out `feat/auth-cleanup` in a slot, so the handle cannot be used: deleting
/// `workspace-1` there would delete a branch nobody asked about and report that it had done
/// the right thing. The fact answers the question because a handler runs on the core task and
/// may not call git; the job answers the act, on the blocking task, from the worktree as it is
/// at that moment.
///
/// The two are reconciled where they can be: `expected_branch` carries what the question named
/// into the job, and the job refuses when the worktree has moved since. **That holds for a key
/// and not for a shell**, whose `--yes` is a second process with nothing to compare against, so
/// a shell reader can be told one branch and lose a different one. The one lost is always the
/// branch the worktree is really on, so this is misinformation rather than misdeletion, and the
/// result names what went. `CoreJob::DeleteWorkspace::expected_branch` has the whole of it.
pub fn delete(ctx: &mut Ctx, p: WorkspaceDeleteParams) -> Result<Value, ApiError> {
    let (target, doomed) = target_of(ctx, Some(&p.workspace), MAIN_CANNOT_BE_DELETED)?;
    if !p.yes {
        if ctx.from_key {
            return ask(ctx, ConfirmKind::DeleteWorkspace(target));
        }
        let copy = deletion_copy(
            &doomed.name,
            &doomed.root,
            &doomed.path,
            doomed.branch.as_deref(),
            doomed.tabs,
        );
        // `needs_confirmation` appends "Answer with --yes" to the message and puts the same
        // content in `data` as lists, so the sentence and the lists cannot disagree.
        return Err(ApiError::needs_confirmation(
            copy.question(),
            copy.removes_list,
            copy.keeps_list,
        ));
    }
    ctx.jobs.push(CoreJob::DeleteWorkspace {
        workspace: target,
        name: doomed.name,
        root: doomed.root,
        path: doomed.path,
        // What the question named, so the job can refuse if the worktree has moved since.
        // Read here rather than in the job because a job may not touch the fact registry, and
        // read now rather than when the box was drawn because this call is the consent: the
        // `--yes` retry and the overlay's `y` both come back through here.
        expected_branch: doomed.branch,
        base: doomed.base,
        force: p.force,
    });
    ctx.defer_reply = true;
    // Discarded, for the reason `clear` gives above.
    ok(Ack { ok: true })
}

/// Puts the question on the screen the key was pressed on and gives it the keys.
///
/// `push_overlay`, not an assignment: the switcher is the one place a destructive key can be
/// pressed while something else is open, and the reader has to get back to it
/// (interface spec 12.7). `input::pop_confirmation` is the other half.
fn ask(ctx: &mut Ctx, kind: ConfirmKind) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    // Reachable, and tested: `Core::run_action` never checks that the client id it is handed
    // is attached, so a key press can arrive carrying one the model has dropped.
    //
    // It is **not** `Ctx::view`'s fallback that this guards, which an earlier version of this
    // comment claimed. `view` is `self.client.clone().or_else(most_recent_client)` and
    // `run_action` always passes `Some(client)`, so on the only path `ask` is reachable from
    // the `or_else` never evaluates. What the refusal buys is that a lookup miss is answered
    // instead of passed over: a handler that resolved a dropped id to some other view would
    // put a question about a workspace nobody named on that reader's screen, and answer `ok`
    // on behalf of a client that is not there.
    let Some(view) = ctx.model.client_mut(&client) else {
        return Err(ApiError::not_found(format!(
            "client {client} is not attached"
        )));
    };
    view.push_overlay(Overlay::Confirm(kind));
    view.focus = Focus::Region(RegionKind::Overlay);
    // Equivalent, and left in on purpose. `Core::key` marks the view after every `route_key`
    // and `ask` is reachable only behind `from_key`, so no frame depends on this line and no
    // test can tell it from its absence. It stays because `api::project::remove` writes the
    // same line on the same path, and one of the two opening a question without saying the
    // screen changed would be the odd one out the day either becomes reachable another way.
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Every exited record in the workspace, resumed: its relaunch line typed into the pane it
/// last ran in, in the order the Agents box lists them (newest first).
///
/// Failures are collected rather than fatal (plan assumption 29). One Codex record in a
/// workspace must not stop the Claude records beside it from coming back, and the reader has to
/// be told which ones did not, so both halves of the answer travel: `resumed` is what was typed
/// and `skipped` is one line per record with the reason (principle 9).
///
/// Every refusal is `agent::plan_resume`'s, which is what `agent.resume` refuses with too, so a
/// record skipped here and the same record named on its own give the same reason in the same
/// words. Nothing about resuming one agent is repeated here.
///
/// **One line per pane.** Two exited records name one pane whenever two sessions ran there in
/// turn, because a record keeps its `last_pane` when it exits. Typing both lines would put the
/// second into whatever the first started, and reporting both in `resumed` would claim a line
/// reached a shell when it reached an agent's prompt (principle 4). So the first record resumed
/// into a pane takes it and the rest are skipped.
///
/// First in the order this loop reads, which is `Model::sorted_agents` - the order the Agents box
/// shows. Choosing *which* record that is belongs to that function and not to this one: it orders
/// exited records by last activity descending, so the one that takes the pane is the session you
/// had last, and V1 chose one session per window for that same reason
/// (`bestAgentSession`, commit b02a3ae).
///
/// A pane is claimed where the line is actually typed and nowhere earlier. A record that cannot
/// resume must not take a pane from one that can: an exited Codex record in front of an exited
/// Claude record in the same pane refuses on its kind, and the Claude record behind it still
/// comes back.
pub fn resume(ctx: &mut Ctx, p: WorkspaceTargetParams) -> Result<Value, ApiError> {
    let workspace = ctx.resolve_workspace_param(p.workspace.as_deref())?;
    // Read out as ids before the loop, because the loop writes to the panes through `ctx` and
    // cannot hold a borrow of the model across that.
    let exited: Vec<domux_core::ids::AgentId> = ctx
        .model
        .sorted_agents()
        .into_iter()
        .filter(|a| a.workspace == workspace && !a.state.is_live())
        .map(|a| a.id.clone())
        .collect();
    let mut resumed = Vec::new();
    let mut skipped = Vec::new();
    let mut typed_into: HashSet<PaneId> = HashSet::new();
    for agent in exited {
        match super::agent::plan_resume(ctx, &agent) {
            Ok((command, pane)) => {
                if typed_into.contains(&pane) {
                    skipped.push(format!(
                        "{agent}: pane {pane} already took a relaunch line in this resume; one session comes back per pane, so resume this one yourself once that pane is free"
                    ));
                    continue;
                }
                match ctx.write_to_pane(&pane, format!("{command}\r").as_bytes()) {
                    Ok(()) => {
                        typed_into.insert(pane.clone());
                        resumed.push(AgentResumeResult {
                            agent,
                            pane,
                            command,
                        })
                    }
                    // A pane with no terminal is a record that cannot be resumed like any other, so
                    // it joins the skipped list instead of ending the run. `plan_resume` has already
                    // refused the pane the model does not hold; this is the narrower case of a pane
                    // the model holds whose process failed to start. It claims no pane: nothing was
                    // typed, so a later record naming the same pane is free to try.
                    Err(e) => skipped.push(format!("{agent}: {}", e.message)),
                }
            }
            Err(e) => skipped.push(format!("{agent}: {}", e.message)),
        }
    }
    ok(WorkspaceResumeResult { resumed, skipped })
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

    /// The workspace the calling client is in - or, while its keys are in a Projects box, the
    /// row its cursor is on.
    ///
    /// That one rule is what makes `leader N` and `n` on a row the same operation (interface
    /// spec 7.1). Neither key carries a target, so the answer has to come from where the keys
    /// are, and `projects_cursor` is the model's own answer to "the row the keys act on". It
    /// is also the key both renderers fill their row from, so the workspace this names is the
    /// one the reader can see the fill on.
    ///
    /// `list::in_a_projects_box` rather than the focus kind: the keys are in the switcher's
    /// box whenever the switcher is open, whatever `focus` holds after an overlay over it
    /// closed, and they are in the sidebar's only while the sidebar is actually showing.
    /// Asking the question `list.*` asks keeps the box the cursor belongs to and the box the
    /// keys are in one answer. The Agents box holds the same keys and is not one of these:
    /// `projects_cursor` names no row the reader can see while the agents overlay is open.
    ///
    /// One state has no fill to point at: a filter that dropped the cursor's row. This still
    /// answers with that row, where `list.activate` refuses. Switching would move the reader
    /// to a workspace the box is not showing; the name box puts the handle in its own title,
    /// so it says which workspace it is naming whether or not a row is drawn for it.
    pub fn workspace_of_view(&self) -> Result<WorkspaceId, ApiError> {
        let client = self.view()?;
        let view = self
            .model
            .client(&client)
            .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
        match &view.projects_cursor {
            Some(cursor) if super::list::in_a_projects_box(self, &client) => Ok(cursor.clone()),
            _ => Ok(view.workspace.clone()),
        }
    }

    /// The project of the row the cursor is on, for a call that named none and must not
    /// guess.
    ///
    /// `project_of_view` falls back to the caller's own project, which is right for creating
    /// a workspace and wrong for removing one: a shell that typed `project remove` with
    /// nothing after it would then take away the project it happens to be in. So this refuses
    /// unless the keys are in a Projects box, where the row is on the screen with the fill on
    /// it, and names the two ways to ask in the refusal.
    pub fn project_of_cursor(&self) -> Result<ProjectId, ApiError> {
        let client = self.view()?;
        if !super::list::in_a_projects_box(self, &client) {
            return Err(ApiError::invalid_params(
                "name a project to remove, or pass --all to remove every one",
            ));
        }
        self.project_of_view()
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
