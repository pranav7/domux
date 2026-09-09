//! `import ...`: read another tool's state and create here what it describes.
//!
//! `import v1` reads V1's session files and creates the projects, workspaces, names and tab
//! names they hold. It opens V1's files read only and writes nothing under V1's state
//! directory: the planning is `domux_core::import_v1`, which cannot open a file at all, and
//! everything this file writes goes through the same API calls a person could type.

use super::{call, call_as, not_running, print_line, socket};
use anyhow::Context;
use clap::{Args, Subcommand};
use domux_client::control;
use domux_core::api::{TabInfo, WorkspaceInfo};
use domux_core::ids::WorkspaceId;
use domux_core::import_v1::{
    self, ImportPlan, PlannedProject, PlannedTab, PlannedWorkspace, Run, Skip, V1Session,
};
use domux_core::paths;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct ImportCmd {
    #[command(subcommand)]
    pub action: ImportAction,
}

#[derive(Subcommand)]
pub enum ImportAction {
    /// Create the projects, workspaces, names and tab names V1's sessions hold
    V1(V1Cmd),
}

#[derive(Args)]
pub struct V1Cmd {
    /// Print what the import would create and change nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Read V1's session files from this directory rather than from V1's own
    #[arg(long, value_name = "DIR")]
    pub from: Option<PathBuf>,
}

pub async fn run(cmd: ImportCmd) -> anyhow::Result<()> {
    let ImportAction::V1(args) = cmd.action;
    let dir = args.from.unwrap_or_else(paths::v1_sessions_dir);
    let (sessions, unreadable) = read_sessions(&dir)?;
    for path in &unreadable {
        eprintln!(
            "Could not read {}, so the session in it was not imported.",
            path.display()
        );
    }
    let plan = import_v1::plan(&sessions, &|p| p.is_dir());

    let (run, projects, lost) = if args.dry_run {
        // Nothing is called, so nothing can refuse: what the plan holds is what the report
        // describes, and its summary says so in its own words rather than in a note beside
        // it.
        (Run::Dry, plan.projects.clone(), Lost::default())
    } else {
        let (projects, lost) = apply(&plan).await?;
        (Run::Real, projects, lost)
    };
    report(run, &projects, &plan.skips)?;

    // A skip is a decision the plan made and said out loud, so it is not a failure. A file
    // that would not read, a project the server would not take and a planned workspace that
    // never arrived are failures, and a failure leaves a status of 1 (principle 12),
    // whatever else the run managed.
    let lost = Lost {
        unreadable: unreadable.len(),
        ..lost
    };
    match import_v1::failure_line(lost.unreadable, lost.projects, lost.workspaces) {
        None => Ok(()),
        Some(line) => Err(anyhow::anyhow!(line)),
    }
}

/// What a run could not carry across. Each field counts its own kind, because one project
/// can hold several sessions and adding them together states a number of one noun about
/// another.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Lost {
    unreadable: usize,
    projects: usize,
    workspaces: usize,
}

/// V1's session files and the paths of the ones that would not read.
///
/// The order they come back in does not matter. `read_dir` promises none, and
/// `import_v1::plan` sorts the sessions itself so that every part of the plan, the skips
/// included, is settled by the sessions rather than by the filesystem. Sorting here as well
/// would look like the guarantee while the real one lived elsewhere.
fn read_sessions(dir: &Path) -> anyhow::Result<(Vec<V1Session>, Vec<PathBuf>)> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    let mut sessions = Vec::new();
    let mut unreadable = Vec::new();
    for path in files {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| import_v1::parse_session(&text).ok())
        {
            Some(session) => sessions.push(session),
            None => unreadable.push(path),
        }
    }
    Ok((sessions, unreadable))
}

/// Creates what the plan describes, and answers with the part of the plan that arrived and
/// the number of projects the server would not take.
///
/// `project.add` is the one call whose failure is an ordinary outcome: a root that is no
/// longer a repository, or one this user may not read. It is reported and the import moves
/// on to the next project. Every call after it acts on records the server has just made, so
/// a failure there means something is wrong rather than something is missing, and it ends
/// the run.
async fn apply(plan: &ImportPlan) -> anyhow::Result<(Vec<PlannedProject>, Lost)> {
    // Asked once, before the loop. Every `project.add` below reports its own failure and
    // carries on, which is right for a failure about that project's root. A server that is
    // not listening is not about any root and is the same answer for all of them, so
    // without this the most likely failure the author will ever hit prints one identical
    // line per project.
    if !control::is_live(&socket()).await {
        return Err(not_running());
    }
    let mut arrived = Vec::new();
    let mut lost = Lost::default();
    for project in &plan.projects {
        // `project.add` adopts the worktrees already on disk, so a slot the plan names is
        // usually there already and only `main` is new. It is also how a project that is
        // registered already is recognised, which is what makes a second import quiet.
        if let Err(e) = call("project.add", json!({ "path": project.root })).await {
            eprintln!("Could not add {}: {e:#}", project.root.display());
            lost.projects += 1;
            continue;
        }
        let registered: Vec<WorkspaceInfo> = call_as("workspace.list", json!({})).await?;
        let mut workspaces = Vec::new();
        for planned in &project.workspaces {
            // By path, not by handle: a handle is unique inside its project and
            // `workspace.rename` resolving `workspace-1` across two projects is ambiguous,
            // which is exactly what an import of several projects produces.
            let Some(found) = registered
                .iter()
                .find(|w| same_directory(&w.path, &planned.path))
            else {
                eprintln!(
                    "{} is not registered under {}, so its tabs were not created.",
                    planned.path.display(),
                    project.root.display()
                );
                lost.workspaces += 1;
                continue;
            };
            if let Some(name) = &planned.name {
                call(
                    "workspace.rename",
                    json!({ "workspace": found.id, "name": name }),
                )
                .await?;
            }
            let tabs = apply_tabs(&found.id, &planned.tabs).await?;
            workspaces.push(PlannedWorkspace {
                tabs,
                ..planned.clone()
            });
        }
        arrived.push(PlannedProject {
            root: project.root.clone(),
            workspaces,
        });
    }
    Ok((arrived, lost))
}

/// Creates the tabs the workspace does not have yet, and answers with every planned tab the
/// workspace now holds, whether this run made it or found it.
///
/// A tab's identity here is the pair of its name and its directory, with an absent name
/// matching only an absent name. Not the index, because V1's `index` is a position and V2's
/// tabs renumber when one closes, so matching on it would duplicate every tab after the
/// first close. Not the name alone, because V1 lets two windows of one session share a name
/// and collapsing them would lose one. Each match is consumed rather than tested for
/// membership, so two identical planned tabs are two tabs: that is what makes a second
/// `import v1` add nothing while an honest duplicate still arrives.
async fn apply_tabs(
    workspace: &WorkspaceId,
    planned: &[PlannedTab],
) -> anyhow::Result<Vec<PlannedTab>> {
    let mut have = tabs_of(workspace).await?;
    let mut arrived = Vec::new();
    for tab in planned {
        let already = have
            .iter()
            .position(|(name, cwd)| name == &tab.name && same_directory(cwd, &tab.cwd));
        match already {
            Some(i) => {
                have.remove(i);
            }
            None => {
                // Named in the create rather than by a `tab.rename` after it. `tab.rename`
                // resolves its target inside the calling client's workspace, and an import
                // runs from a shell with no pane, so it could not name a tab it had just
                // made in a workspace nobody is looking at.
                let _made: TabInfo = call_as(
                    "tab.create",
                    json!({ "workspace": workspace, "cwd": tab.cwd, "name": tab.name }),
                )
                .await?;
            }
        }
        arrived.push(tab.clone());
    }
    Ok(arrived)
}

/// A workspace's tabs as the pair the comparison above needs: each tab's name and its
/// directory.
///
/// The directory a tab reports is where its shell is now, not where the tab was opened,
/// because the server polls it from the pane as it runs. So a tab whose shell has moved
/// reads as a different tab and a second import creates it again. Nothing in V2 records the
/// directory a tab was opened at, so this is the closest question that can be asked.
async fn tabs_of(workspace: &WorkspaceId) -> anyhow::Result<Vec<(Option<String>, PathBuf)>> {
    let tabs: Vec<TabInfo> = call_as("tab.list", json!({ "workspace": workspace })).await?;
    Ok(tabs.into_iter().map(|tab| (tab.name, tab.cwd)).collect())
}

/// Whether two names reach the same directory. V1's file holds the path the author typed and
/// the server answers with the path it resolved, so `/tmp/x` on a Mac is `/private/tmp/x` in
/// the answer and comparing the two as written would find nothing. A path that will not
/// resolve is compared as it stands.
fn same_directory(a: &Path, b: &Path) -> bool {
    resolved(a) == resolved(b)
}

fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The plan, then what it comes to, then what did not come across. The same lines in a dry
/// run and in a real one, so the author can hold the two side by side.
fn report(run: Run, projects: &[PlannedProject], skips: &[Skip]) -> anyhow::Result<()> {
    for line in import_v1::plan_lines(projects) {
        print_line(&line)?;
    }
    let (projects, workspaces, tabs) = import_v1::counts(projects);
    print_line(&import_v1::report_line(run, projects, workspaces, tabs))?;
    if let Some(line) = import_v1::skipped_line(skips) {
        print_line(&line)?;
    }
    Ok(())
}
