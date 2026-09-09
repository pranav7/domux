//! V1's session files, and the plan V2 would create from them.
//!
//! V1 keeps one JSON object per tmux session in its own state directory, written by
//! `session.go`: `name`, `root`, `label` (V1's word for what V2 calls a name), `workspace`,
//! `windows` with `index`, `name`, `cwd` and `agent`, `created_at` and `updated_at`. This
//! module reads that shape and says what V2 would create from it.
//!
//! Everything here is pure. It never opens a file, so it cannot write one: `import v1`
//! reads V1's files and nothing in V2 writes under V1's state directory.
//!
//! The two models do not line up everywhere, and where they do not the plan carries a
//! `Skip` with its reason rather than dropping the session quietly. A skip the author
//! cannot see is worse than a failure.

use crate::model::WorkspaceHandle;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Where a project's worktrees live, and the name V1 used before the rename. Both are read,
/// because a project the author has been using with V1 has its slots under whichever of the
/// two V1 made them under.
///
/// `domux_server::git::WORKTREE_DIR` and `LEGACY_WORKTREE_DIR` hold the same two strings and
/// are the ones `project.add` reads. They are not shared, because this crate is the pure
/// model and may not depend on the server, and nothing checks that the two pairs agree. If
/// the server's ever change, change these by hand.
const WORKTREE_DIRS: [&str; 2] = [".domux/worktrees", ".baag/worktrees"];

/// One of V1's session files. Every field is defaulted: a file written by an older V1 is
/// missing half of them, and a file written by a newer one carries fields V2 has no use for
/// (`todo_path`), which are ignored rather than refused.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct V1Session {
    /// V1's name for the session, which is the tmux session name.
    pub name: String,
    pub root: Option<PathBuf>,
    /// V1's word for what V2 calls a name.
    pub label: Option<String>,
    pub workspace: Option<String>,
    pub windows: Vec<V1Window>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// One of a V1 session's windows. "window" is V1's own field name, quoted here because this
/// is a description of V1's file; V2's word for what a window becomes is tab.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct V1Window {
    /// V1's position in the session, which decides the order the tabs are planned in.
    pub index: u32,
    pub name: String,
    pub cwd: Option<PathBuf>,
    pub agent: Option<String>,
}

/// What V2 would create, and what it would not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportPlan {
    pub projects: Vec<PlannedProject>,
    pub skips: Vec<Skip>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedProject {
    pub root: PathBuf,
    pub workspaces: Vec<PlannedWorkspace>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedWorkspace {
    /// `main`, or `workspace-1`: what V2 calls the workspace once `project.add` has adopted
    /// the directory.
    pub handle: String,
    pub path: PathBuf,
    /// V1's `label`, when V2 can hold it as a name.
    pub name: Option<String>,
    pub tabs: Vec<PlannedTab>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedTab {
    pub name: Option<String>,
    pub cwd: PathBuf,
}

/// Something V1 recorded that did not come across, and why. The author reads these as
/// `audrey-app (root is gone)`, so `reason` is a sentence fragment that follows the name.
#[derive(Debug, Clone, PartialEq)]
pub struct Skip {
    /// Which of V1's sessions this came from, as the author would recognise it.
    pub session: String,
    pub reason: String,
}

/// A session's file with no `root` at all, or with an empty one.
pub const NO_ROOT: &str = "no root recorded";
/// A session whose root the caller's `exists` predicate says is not there.
pub const ROOT_IS_GONE: &str = "root is gone";

/// Reads one of V1's session files. Every field defaults, so the only failure is text that
/// is not a JSON object at all.
///
/// The object check is not redundant. serde reads a struct from a JSON sequence as well as
/// from a map, so `[]` would otherwise parse as a session with every field defaulted, and a
/// file that is not a session at all would reach the author as a session with no root.
pub fn parse_session(text: &str) -> Result<V1Session, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    if !value.is_object() {
        return Err(serde::de::Error::custom(
            "a V1 session file is a JSON object",
        ));
    }
    serde_json::from_value(value)
}

/// What V2 would create from V1's sessions.
///
/// `exists` decides whether a session's root is still on disk, so the caller owns every
/// filesystem question and this function stays pure. `import v1` passes `Path::is_dir`.
pub fn plan(sessions: &[V1Session], exists: &dyn Fn(&Path) -> bool) -> ImportPlan {
    let mut projects: Vec<Building> = Vec::new();
    let mut skips: Vec<Skip> = Vec::new();

    // Sorted here rather than by whoever read the files, so every output of this function is
    // settled by the sessions themselves. Two of them decide something by order: which of two
    // sessions naming one workspace keeps its name, and the order of the skips. Leaving that
    // to the caller made it depend on `read_dir`, which promises no order, and no test could
    // pin it without depending on the same thing.
    let mut sessions: Vec<&V1Session> = sessions.iter().collect();
    sessions.sort_by(|a, b| {
        (a.root.as_deref(), a.name.as_str()).cmp(&(b.root.as_deref(), b.name.as_str()))
    });

    for session in sessions {
        let who = session_label(session);
        let Some(root) = session.root.as_ref().filter(|r| !r.as_os_str().is_empty()) else {
            skips.push(Skip {
                session: who,
                reason: NO_ROOT.to_string(),
            });
            continue;
        };
        if !exists(root) {
            skips.push(Skip {
                session: who,
                reason: ROOT_IS_GONE.to_string(),
            });
            continue;
        }

        let (project_root, handle) = match split_worktree(root) {
            None => (root.clone(), WorkspaceHandle::Main),
            Some((project_root, directory)) => match slot_of(&directory) {
                Some(slot) => (project_root, WorkspaceHandle::Slot(slot)),
                None => {
                    skips.push(Skip {
                        session: who,
                        reason: not_a_slot(&directory),
                    });
                    continue;
                }
            },
        };

        let mut name = trimmed(session.label.as_deref());
        if let Some(text) = &name {
            // V2 refuses a name that reads as a handle, because the handle pass of
            // `Model::resolve_workspace_with` runs first and such a name could never resolve
            // to the workspace it was given to. The workspace still comes across.
            if WorkspaceHandle::reads_as_handle(text) {
                skips.push(Skip {
                    session: who.clone(),
                    reason: label_reads_as_handle(text),
                });
                name = None;
            }
        }

        let mut windows: Vec<&V1Window> = session.windows.iter().collect();
        // A stable sort, so two windows V1 gave the same index keep the order the file has.
        windows.sort_by_key(|w| w.index);
        let mut tabs = Vec::new();
        for window in windows {
            if let Some(agent) = trimmed(window.agent.as_deref()) {
                skips.push(Skip {
                    session: who.clone(),
                    reason: agent_has_no_home(&window.name, &agent),
                });
            }
            tabs.push(PlannedTab {
                name: trimmed(Some(&window.name)),
                // A window V1 recorded no directory for opens where its workspace is. That
                // is the only directory V2 knows of for it, and a tab has to start somewhere.
                cwd: window
                    .cwd
                    .clone()
                    .filter(|c| !c.as_os_str().is_empty())
                    .unwrap_or_else(|| root.clone()),
            });
        }

        let project = match projects.iter_mut().find(|p| p.root == project_root) {
            Some(p) => p,
            None => {
                projects.push(Building {
                    root: project_root,
                    workspaces: Vec::new(),
                });
                projects.last_mut().expect("just pushed")
            }
        };
        match project.workspaces.iter_mut().find(|(h, _)| *h == handle) {
            // Two of V1's sessions can name one workspace. Their windows join in one
            // workspace rather than one of them going quietly, and only the first name is
            // taken, because a workspace has one.
            Some((_, existing)) => {
                match (&existing.name, &name) {
                    (None, Some(_)) => existing.name = name,
                    (Some(held), Some(offered)) if held != offered => skips.push(Skip {
                        session: who,
                        reason: name_already_taken(&existing.handle, &project.root, offered),
                    }),
                    _ => {}
                }
                existing.tabs.extend(tabs);
            }
            None => project.workspaces.push((
                handle,
                PlannedWorkspace {
                    handle: handle.to_string(),
                    path: root.clone(),
                    name,
                    tabs,
                },
            )),
        }
    }

    // The projects are already in root order: the sessions were sorted by root above, and a
    // project root is an ancestor of its session root, so the order the projects were first
    // seen in is the order their roots sort in. An explicit sort here used to say so and did
    // nothing, which is a line no test could ever fail without.
    ImportPlan {
        projects: projects
            .into_iter()
            .map(|mut building| {
                building.workspaces.sort_by_key(|(handle, _)| match handle {
                    WorkspaceHandle::Main => (0, 0),
                    WorkspaceHandle::Slot(n) => (1, *n),
                });
                PlannedProject {
                    root: building.root,
                    workspaces: building.workspaces.into_iter().map(|(_, w)| w).collect(),
                }
            })
            .collect(),
        skips,
    }
}

/// A project while the plan is being built, with each workspace's handle beside it so the
/// sort at the end does not have to read the handle back out of its own printed form.
struct Building {
    root: PathBuf,
    workspaces: Vec<(WorkspaceHandle, PlannedWorkspace)>,
}

/// How many projects, workspaces and tabs a set of planned projects comes to.
pub fn counts(projects: &[PlannedProject]) -> (usize, usize, usize) {
    let workspaces = projects.iter().map(|p| p.workspaces.len()).sum();
    let tabs = projects
        .iter()
        .flat_map(|p| p.workspaces.iter())
        .map(|w| w.tabs.len())
        .sum();
    (projects.len(), workspaces, tabs)
}

/// Whether a summary describes what an import did or what it would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// The import called the server.
    Real,
    /// `--dry-run`: nothing was called.
    Dry,
}

/// What the import did, or would do, in one line.
///
/// The tense is in this sentence rather than in a note beside it, because a note can be on
/// another stream or redirected away and this line cannot. `import v1 --dry-run > plan.txt`
/// used to leave a file saying "Imported 3 tabs" about work that never happened, which is a
/// fact that did not arrive rendered as one (principle 4).
///
/// `skipped_line` needs no such tense. A skip happens when the plan is made, so a dry run
/// really has skipped what it names, and a real run skipped it before it called anything.
pub fn report_line(run: Run, projects: usize, workspaces: usize, tabs: usize) -> String {
    let verb = match run {
        Run::Real => "Imported",
        Run::Dry => "Would import",
    };
    format!(
        "{verb} {}, {}, {}.",
        count_of(projects, "project"),
        count_of(workspaces, "workspace"),
        count_of(tabs, "tab")
    )
}

/// What an import could not carry across, when something failed rather than being skipped.
///
/// Each count names its own noun. The three are different things and a run can lose some of
/// each, so they are never added together: a count of projects presented as a count of
/// sessions is a wrong number in front of the author, and one project can hold several
/// sessions.
pub fn failure_line(unreadable: usize, projects: usize, workspaces: usize) -> Option<String> {
    let mut parts = Vec::new();
    if unreadable > 0 {
        parts.push(format!(
            "{} that would not read",
            count_of(unreadable, "session file")
        ));
    }
    if projects > 0 {
        parts.push(count_of(projects, "project"));
    }
    if workspaces > 0 {
        parts.push(count_of(workspaces, "workspace"));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "Not imported: {}. Read the messages above, then run this again.",
        parts.join(", ")
    ))
}

/// What did not come across, in one line, or nothing at all when everything did.
pub fn skipped_line(skips: &[Skip]) -> Option<String> {
    if skips.is_empty() {
        return None;
    }
    let each: Vec<String> = skips
        .iter()
        .map(|s| format!("{} ({})", s.session, s.reason))
        .collect();
    Some(format!("Skipped {}: {}.", skips.len(), each.join(", ")))
}

/// The plan, one line per project root and one indented line per workspace under it. This is
/// what the author reads before running the import for real, so it names every workspace and
/// every tab rather than counting them.
pub fn plan_lines(projects: &[PlannedProject]) -> Vec<String> {
    let mut out = Vec::new();
    for project in projects {
        out.push(project.root.display().to_string());
        for workspace in &project.workspaces {
            let mut line = format!("  {}", workspace.handle);
            if let Some(name) = &workspace.name {
                line.push_str(&format!(", named {name}"));
            }
            if !workspace.tabs.is_empty() {
                let names: Vec<&str> = workspace
                    .tabs
                    .iter()
                    .map(|t| t.name.as_deref().unwrap_or(UNNAMED_TAB))
                    .collect();
                line.push_str(&format!(", tabs: {}", names.join(", ")));
            }
            out.push(line);
        }
    }
    out
}

/// What a tab with no name reads as in the plan. V2 draws such a tab as its number, which
/// the plan does not know yet.
pub const UNNAMED_TAB: &str = "(unnamed)";

fn count_of(n: usize, thing: &str) -> String {
    match n {
        1 => format!("1 {thing}"),
        n => format!("{n} {thing}s"),
    }
}

fn session_label(session: &V1Session) -> String {
    if let Some(name) = trimmed(Some(&session.name)) {
        return name;
    }
    // A session file with no name is still one the author has to be able to find, so the
    // skip names its root instead.
    match session.root.as_ref().filter(|r| !r.as_os_str().is_empty()) {
        Some(root) => root.display().to_string(),
        None => "a session with no name".to_string(),
    }
}

fn trimmed(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn not_a_slot(directory: &str) -> String {
    format!(
        "its worktree directory is called {directory}, and V2 adopts only workspace-1, \
         workspace-2 and so on"
    )
}

fn label_reads_as_handle(label: &str) -> String {
    format!("its label {label} reads as a handle, so the workspace came across without a name")
}

fn agent_has_no_home(window: &str, agent: &str) -> String {
    let window = trimmed(Some(window)).unwrap_or_else(|| UNNAMED_TAB.to_string());
    format!("its window {window} runs the agent {agent}, which V2 has no home for until M3")
}

fn name_already_taken(handle: &str, root: &Path, offered: &str) -> String {
    format!(
        "another session already names {handle} in {}, so the name {offered} was not imported",
        root.display()
    )
}

/// The project a worktree directory belongs to, and the directory's own name, when the root
/// sits under one of the worktree directories. `None` for a root anywhere else, which is a
/// main checkout.
fn split_worktree(root: &Path) -> Option<(PathBuf, String)> {
    let directory = root.file_name()?.to_str()?.to_string();
    let parent = root.parent()?;
    for worktrees in WORKTREE_DIRS {
        if let Some(project) = without_trailing(parent, worktrees) {
            return Some((project, directory));
        }
    }
    None
}

/// `path` with the components of `trailing` taken off its end, when they are there.
fn without_trailing(path: &Path, trailing: &str) -> Option<PathBuf> {
    let mut left = path.to_path_buf();
    for part in Path::new(trailing).components().rev() {
        if left.file_name()? != part.as_os_str() {
            return None;
        }
        left = left.parent()?.to_path_buf();
    }
    Some(left)
}

/// The slot a worktree directory stands for.
///
/// This is the grammar `domux_server::git::existing_slots` reads, which decides whether
/// `project.add` adopts the directory at all, plus the round trip through `Display` that
/// `WorkspaceHandle` uses to settle the edge cases: `workspace-01` parses as a number but no
/// handle prints it. Planning a handle that adoption would never produce is planning a
/// `workspace.rename` that lands nowhere, so the plan refuses the directory instead.
fn slot_of(directory: &str) -> Option<u32> {
    let slot: u32 = directory.strip_prefix("workspace-")?.parse().ok()?;
    (slot >= 1 && WorkspaceHandle::Slot(slot).to_string() == directory).then_some(slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str) -> V1Session {
        parse_session(text).expect("a fixture parses")
    }

    fn audrey_app() -> V1Session {
        fixture(include_str!("../fixtures/import/audrey-app.json"))
    }
    fn workspace_1() -> V1Session {
        fixture(include_str!("../fixtures/import/workspace-1.json"))
    }
    fn legacy() -> V1Session {
        fixture(include_str!("../fixtures/import/legacy.json"))
    }
    fn nameless() -> V1Session {
        fixture(include_str!("../fixtures/import/nameless.json"))
    }
    fn no_root() -> V1Session {
        fixture(include_str!("../fixtures/import/no-root.json"))
    }
    fn handle_label() -> V1Session {
        fixture(include_str!("../fixtures/import/handle-label.json"))
    }
    fn not_a_slot_session() -> V1Session {
        fixture(include_str!("../fixtures/import/not-a-slot.json"))
    }
    fn older() -> V1Session {
        fixture(include_str!("../fixtures/import/older.json"))
    }
    fn out_of_order() -> V1Session {
        fixture(include_str!("../fixtures/import/out-of-order.json"))
    }
    fn duplicate() -> V1Session {
        fixture(include_str!("../fixtures/import/duplicate.json"))
    }

    /// A session rooted at `root` with nothing else set, for the cases no fixture is worth.
    fn rooted(name: &str, root: &str) -> V1Session {
        V1Session {
            name: name.to_string(),
            root: Some(PathBuf::from(root)),
            ..V1Session::default()
        }
    }

    /// The plan with everything on disk.
    fn planned(sessions: &[V1Session]) -> ImportPlan {
        plan(sessions, &|_| true)
    }

    fn tab_names(w: &PlannedWorkspace) -> Vec<Option<String>> {
        w.tabs.iter().map(|t| t.name.clone()).collect()
    }

    fn handles(p: &PlannedProject) -> Vec<&str> {
        p.workspaces.iter().map(|w| w.handle.as_str()).collect()
    }

    fn reasons(plan: &ImportPlan) -> Vec<&str> {
        plan.skips.iter().map(|s| s.reason.as_str()).collect()
    }

    #[test]
    fn a_worktree_root_plans_a_slot_in_the_project_above_it() {
        let plan = planned(&[workspace_1()]);
        assert_eq!(plan.projects.len(), 1);
        let p = &plan.projects[0];
        assert_eq!(p.root, PathBuf::from("/repo/audrey-app"));
        assert_eq!(p.workspaces[0].handle, "workspace-1");
        assert_eq!(
            p.workspaces[0].path,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1")
        );
        assert_eq!(
            tab_names(&p.workspaces[0]),
            vec![Some("comments".into()), Some("server".into())]
        );
        assert_eq!(
            p.workspaces[0].tabs[1].cwd,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1/frontend")
        );
    }

    #[test]
    fn a_worktree_root_under_the_name_v1_used_before_the_rename_plans_a_slot_too() {
        let plan = planned(&[legacy()]);
        let p = &plan.projects[0];
        assert_eq!(p.root, PathBuf::from("/repo/audrey-app"));
        assert_eq!(p.workspaces[0].handle, "workspace-2");
        assert_eq!(
            p.workspaces[0].path,
            PathBuf::from("/repo/audrey-app/.baag/worktrees/workspace-2")
        );
    }

    #[test]
    fn a_root_anywhere_else_plans_that_projects_main() {
        let plan = planned(&[audrey_app()]);
        let p = &plan.projects[0];
        assert_eq!(p.root, PathBuf::from("/repo/audrey-app"));
        assert_eq!(p.workspaces.len(), 1);
        assert_eq!(p.workspaces[0].handle, "main");
        assert_eq!(p.workspaces[0].path, PathBuf::from("/repo/audrey-app"));
    }

    /// The worktree directory is a pair of names, not the word `worktrees` on its own. A
    /// root with a `worktrees` directory under neither dot directory is a checkout of its
    /// own, and reading only the last component would plan its project one level too high.
    #[test]
    fn a_worktrees_directory_under_no_dot_directory_is_a_main_checkout() {
        let plan = planned(&[rooted("odd", "/repo/odd/worktrees/workspace-1")]);
        assert_eq!(
            plan.projects[0].root,
            PathBuf::from("/repo/odd/worktrees/workspace-1")
        );
        assert_eq!(handles(&plan.projects[0]), vec!["main"]);
    }

    #[test]
    fn the_label_becomes_the_workspaces_name() {
        let plan = planned(&[audrey_app()]);
        assert_eq!(
            plan.projects[0].workspaces[0].name.as_deref(),
            Some("audit-harness")
        );
    }

    #[test]
    fn an_empty_label_plans_no_name() {
        let plan = planned(&[nameless()]);
        assert_eq!(plan.projects[0].workspaces[0].name, None);
    }

    #[test]
    fn an_absent_label_plans_no_name() {
        let plan = planned(&[older()]);
        assert_eq!(plan.projects[0].workspaces[0].name, None);
    }

    #[test]
    fn each_window_becomes_a_tab_in_index_order() {
        let plan = planned(&[out_of_order()]);
        assert_eq!(
            tab_names(&plan.projects[0].workspaces[0]),
            vec![
                Some("first".into()),
                Some("second".into()),
                Some("third".into())
            ]
        );
    }

    #[test]
    fn a_window_with_an_empty_name_plans_an_unnamed_tab() {
        let plan = planned(&[nameless()]);
        let w = &plan.projects[0].workspaces[0];
        // The first window has a directory of its own and no name, so the name is the only
        // field this can be reading.
        assert_eq!(w.tabs[0].name, None);
        assert_eq!(w.tabs[0].cwd, PathBuf::from("/repo/planner/docs"));
    }

    #[test]
    fn a_window_with_no_directory_opens_where_its_workspace_is() {
        let plan = planned(&[nameless()]);
        let w = &plan.projects[0].workspaces[0];
        // The second window has a name and no directory, the mirror of the first, so
        // neither field can stand in for the other.
        assert_eq!(w.tabs[1].name.as_deref(), Some("notes"));
        assert_eq!(w.tabs[1].cwd, PathBuf::from("/repo/planner"));
    }

    #[test]
    fn a_window_with_no_directory_in_a_slot_opens_in_the_slot_not_the_project() {
        let mut session = workspace_1();
        session.windows[0].cwd = None;
        let plan = planned(&[session]);
        assert_eq!(
            plan.projects[0].workspaces[0].tabs[0].cwd,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1")
        );
    }

    #[test]
    fn a_session_with_no_root_is_a_skip_and_plans_nothing() {
        let plan = planned(&[no_root()]);
        assert!(plan.projects.is_empty());
        assert_eq!(plan.skips.len(), 1);
        assert_eq!(plan.skips[0].session, "planner-old");
        assert_eq!(plan.skips[0].reason, "no root recorded");
    }

    #[test]
    fn a_session_with_an_empty_root_is_a_skip_and_plans_nothing() {
        let plan = planned(&[rooted("blank", "")]);
        assert!(plan.projects.is_empty());
        assert_eq!(reasons(&plan), vec!["no root recorded"]);
    }

    #[test]
    fn a_session_whose_root_is_gone_is_a_skip_and_plans_nothing() {
        // Only `/repo/planner` is on disk, so one session comes across and the other does
        // not. A predicate answering false for everything would also pass a plan that
        // skipped everything, which is why both sessions are here.
        let plan = plan(&[audrey_app(), nameless()], &|p| {
            p == Path::new("/repo/planner")
        });
        assert_eq!(plan.projects.len(), 1);
        assert_eq!(plan.projects[0].root, PathBuf::from("/repo/planner"));
        assert_eq!(plan.skips.len(), 1);
        assert_eq!(plan.skips[0].session, "audrey-app");
        assert_eq!(plan.skips[0].reason, "root is gone");
    }

    /// The predicate is asked about the session's own root, not about the project above it.
    /// A slot that is gone from a project that is still there has to skip.
    #[test]
    fn a_slot_that_is_gone_skips_although_its_project_is_there() {
        // `duplicate.json` rather than `audrey-app.json`: it is rooted in the same project
        // and runs no agent, so `root is gone` is the only reason either session can give.
        let plan = plan(&[duplicate(), workspace_1()], &|p| {
            p == Path::new("/repo/audrey-app")
        });
        assert_eq!(handles(&plan.projects[0]), vec!["main"]);
        assert_eq!(reasons(&plan), vec!["root is gone"]);
    }

    #[test]
    fn two_sessions_in_the_same_project_plan_one_project() {
        let plan = planned(&[audrey_app(), workspace_1(), legacy()]);
        assert_eq!(plan.projects.len(), 1);
        assert_eq!(
            handles(&plan.projects[0]),
            vec!["main", "workspace-1", "workspace-2"]
        );
    }

    #[test]
    fn planning_the_same_sessions_twice_plans_the_same_paths() {
        let sessions = [audrey_app(), workspace_1(), nameless()];
        assert_eq!(planned(&sessions), planned(&sessions));
    }

    /// The plan is a set of paths, so the order `read_dir` handed the files over in cannot
    /// change it: the author compares a dry run against the run that follows it.
    #[test]
    fn the_plan_is_the_same_whatever_order_the_sessions_arrive_in() {
        let forwards = planned(&[audrey_app(), workspace_1(), legacy(), nameless()]);
        let backwards = planned(&[nameless(), legacy(), workspace_1(), audrey_app()]);
        assert_eq!(forwards.projects, backwards.projects);
        let roots: Vec<PathBuf> = forwards.projects.iter().map(|p| p.root.clone()).collect();
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/repo/audrey-app"),
                PathBuf::from("/repo/planner")
            ]
        );
        assert_eq!(
            handles(&forwards.projects[0]),
            vec!["main", "workspace-1", "workspace-2"]
        );
    }

    /// The skips settle too, and they are the part the caller cannot sort afterwards,
    /// because a skip's place in the list is the only order it has. Whoever reads V1's
    /// directory gets no order from `read_dir`, so if this were left to them no test could
    /// pin it without depending on the filesystem.
    #[test]
    fn the_skips_come_out_in_the_same_order_whatever_order_the_sessions_arrive_in() {
        let set = [
            no_root(),
            not_a_slot_session(),
            handle_label(),
            audrey_app(),
        ];
        let forwards = planned(&set);
        let mut backwards: Vec<V1Session> = set.to_vec();
        backwards.reverse();
        let backwards = planned(&backwards);
        assert_eq!(forwards.skips, backwards.skips);
        // Four sessions, four different reasons, so no two entries of this list can stand
        // in for each other and a wrong order is visible.
        assert_eq!(
            forwards
                .skips
                .iter()
                .map(|s| s.session.as_str())
                .collect::<Vec<_>>(),
            // Sorted by root, with a session that has none first: `/repo/atlas` and the
            // worktree under it both precede `/repo/audrey-app`, because `atlas` sorts
            // before `audrey-app`. I predicted session order here and the test disproved
            // it, so the prediction went rather than the code.
            vec!["planner-old", "atlas", "atlas-feature", "audrey-app"]
        );
    }

    #[test]
    fn a_field_v2_has_no_use_for_is_ignored_rather_than_refused() {
        let text = include_str!("../fixtures/import/audrey-app.json");
        assert!(text.contains("todo_path"), "the fixture carries the field");
        assert_eq!(parse_session(text).unwrap().name, "audrey-app");
    }

    #[test]
    fn a_file_written_by_an_older_v1_defaults_every_field_it_lacks() {
        let session = older();
        assert_eq!(session.name, "ledger");
        assert_eq!(session.root, Some(PathBuf::from("/repo/ledger")));
        assert_eq!(session.label, None);
        assert_eq!(session.workspace, None);
        assert!(session.windows.is_empty());
        assert_eq!(session.created_at, None);
        assert_eq!(session.updated_at, None);
    }

    #[test]
    fn a_file_that_is_not_a_session_object_fails_to_parse() {
        assert!(parse_session("{").is_err(), "truncated");
        assert!(parse_session("[]").is_err(), "a sequence is not a session");
        assert!(parse_session("null").is_err(), "null is not a session");
        assert!(parse_session(r#"{"windows": 3}"#).is_err(), "a wrong type");
        // And the check refuses only the shape: an object still parses.
        assert!(parse_session("{}").is_ok());
    }

    #[test]
    fn a_label_that_reads_as_a_handle_is_refused_and_the_workspace_still_comes_across() {
        let plan = planned(&[handle_label()]);
        let w = &plan.projects[0].workspaces[0];
        assert_eq!(w.handle, "main");
        assert_eq!(w.name, None);
        assert_eq!(tab_names(w), vec![Some("build".into())]);
        assert_eq!(plan.skips.len(), 1);
        assert_eq!(plan.skips[0].session, "atlas");
        assert_eq!(
            plan.skips[0].reason,
            "its label workspace-2 reads as a handle, so the workspace came across \
             without a name"
        );
    }

    /// A label that only looks like one. `WorkspaceHandle::reads_as_handle` decides, and it
    /// takes `workspace-01` as a name because no handle prints that.
    #[test]
    fn a_label_that_no_handle_could_shadow_is_kept_as_a_name() {
        let mut session = handle_label();
        session.label = Some("workspace-01".into());
        let plan = planned(&[session]);
        assert_eq!(
            plan.projects[0].workspaces[0].name.as_deref(),
            Some("workspace-01")
        );
        assert!(plan.skips.is_empty(), "{:?}", plan.skips);
    }

    #[test]
    fn a_window_with_an_agent_plans_its_tab_and_says_the_agent_did_not_come_across() {
        let plan = planned(&[audrey_app()]);
        assert_eq!(
            tab_names(&plan.projects[0].workspaces[0]),
            vec![Some("agent-harness".into())]
        );
        assert_eq!(
            reasons(&plan),
            vec![
                "its window agent-harness runs the agent claude, which V2 has no home for \
                 until M3"
            ]
        );
    }

    /// The skip has to name the window, and a window with an agent need not have a name.
    /// No fixture pairs the two, so the fallback is built here.
    #[test]
    fn an_agent_in_a_window_with_no_name_still_names_which_window_it_was() {
        let mut session = audrey_app();
        session.windows[0].name = String::new();
        let plan = planned(&[session]);
        assert_eq!(
            reasons(&plan),
            vec![
                "its window (unnamed) runs the agent claude, which V2 has no home for \
                 until M3"
            ]
        );
    }

    /// An empty `agent` is no agent, and that is what V1 writes for a window without one.
    /// Every window of `workspace-1.json` has `"agent": ""`.
    #[test]
    fn a_window_with_an_empty_agent_is_no_skip_at_all() {
        let plan = planned(&[workspace_1()]);
        assert!(plan.skips.is_empty(), "{:?}", plan.skips);
    }

    #[test]
    fn a_worktree_directory_v2_would_never_adopt_is_a_skip_rather_than_a_project() {
        let plan = planned(&[not_a_slot_session()]);
        assert!(plan.projects.is_empty(), "{:?}", plan.projects);
        assert_eq!(plan.skips.len(), 1);
        assert_eq!(plan.skips[0].session, "atlas-feature");
        assert_eq!(
            plan.skips[0].reason,
            "its worktree directory is called feature-x, and V2 adopts only workspace-1, \
             workspace-2 and so on"
        );
    }

    /// `workspace-01` parses as a number and prints as `workspace-1`, so no handle names
    /// that directory and a `workspace.rename` sent to `workspace-1` would land on a slot
    /// the author never saw. `workspace-0` is not a slot either: adoption starts at 1.
    #[test]
    fn a_worktree_directory_that_is_nearly_a_slot_is_still_a_skip() {
        for directory in ["workspace-01", "workspace-0", "workspace-", "workspace-1x"] {
            let root = format!("/repo/atlas/.domux/worktrees/{directory}");
            let plan = planned(&[rooted(directory, &root)]);
            assert!(plan.projects.is_empty(), "{directory} planned a project");
            assert_eq!(plan.skips.len(), 1, "{directory}");
        }
        // A slot number of more than one digit is a slot, so the rule refuses the four
        // above for their shape rather than for their length.
        let plan = planned(&[rooted(
            "twelve",
            "/repo/atlas/.domux/worktrees/workspace-12",
        )]);
        assert_eq!(handles(&plan.projects[0]), vec!["workspace-12"]);
    }

    #[test]
    fn two_sessions_that_name_one_workspace_plan_one_workspace_with_both_sets_of_tabs() {
        let plan = planned(&[audrey_app(), duplicate()]);
        assert_eq!(plan.projects.len(), 1);
        assert_eq!(plan.projects[0].workspaces.len(), 1);
        let w = &plan.projects[0].workspaces[0];
        // Three tabs. The first two share a name and a directory, because V1 really had two
        // windows and collapsing them would lose one. The third is named differently on
        // purpose: with every tab called `agent-harness` the joined order would be
        // indistinguishable from its reverse, and from the second session's tabs replacing
        // the first's rather than following them.
        assert_eq!(
            tab_names(w),
            vec![
                Some("agent-harness".into()),
                Some("agent-harness".into()),
                Some("second look".into())
            ]
        );
        assert_eq!(w.name.as_deref(), Some("audit-harness"));
        assert!(
            reasons(&plan).contains(
                &"another session already names main in /repo/audrey-app, so the name \
                  second opinion was not imported"
            ),
            "{:?}",
            plan.skips
        );
    }

    /// Two sessions with the same root are separated by their names, not left in whatever
    /// order they arrived in.
    ///
    /// `sort_by` is stable, so a sort that compared roots alone would leave two sessions of
    /// one root in the caller's order and the caller's order would decide which name a
    /// workspace keeps. Passing the pair both ways round is the only fixture that can tell
    /// the two sorts apart: with the name in the key, `audrey-app` precedes
    /// `audrey-app-again` whichever way they arrive.
    #[test]
    fn two_sessions_of_one_root_are_ordered_by_name_whichever_way_they_arrive() {
        for pair in [[audrey_app(), duplicate()], [duplicate(), audrey_app()]] {
            let plan = planned(&pair);
            assert_eq!(
                plan.projects[0].workspaces[0].name.as_deref(),
                Some("audit-harness"),
                "the session named audrey-app holds the name either way"
            );
            assert_eq!(
                tab_names(&plan.projects[0].workspaces[0]),
                vec![
                    Some("agent-harness".into()),
                    Some("agent-harness".into()),
                    Some("second look".into())
                ],
                "and its windows come first either way"
            );
        }
    }

    /// The first session had no name and the second does, so the second's is taken rather
    /// than refused. Otherwise the order the files were read in would decide whether the
    /// author's one name survived.
    #[test]
    fn a_second_session_gives_a_workspace_the_name_the_first_did_not() {
        let mut first = duplicate();
        first.label = None;
        let plan = planned(&[first, audrey_app()]);
        assert_eq!(
            plan.projects[0].workspaces[0].name.as_deref(),
            Some("audit-harness")
        );
        assert!(
            !reasons(&plan).iter().any(|r| r.contains("already names")),
            "{:?}",
            plan.skips
        );
    }

    /// Two sessions offering the same name is one name, not a conflict.
    #[test]
    fn two_sessions_offering_one_name_is_no_skip() {
        let mut second = duplicate();
        second.label = Some("audit-harness".into());
        let plan = planned(&[audrey_app(), second]);
        assert_eq!(
            plan.projects[0].workspaces[0].name.as_deref(),
            Some("audit-harness")
        );
        assert!(
            !reasons(&plan).iter().any(|r| r.contains("already names")),
            "{:?}",
            plan.skips
        );
    }

    #[test]
    fn a_session_with_no_name_is_named_by_its_root_in_a_skip() {
        // Empty and whitespace only both count as no name: a skip reading "   (root is
        // gone)" names nothing the author can look for.
        for name in ["", "   "] {
            let mut session = not_a_slot_session();
            session.name = name.to_string();
            let plan = planned(&[session]);
            assert_eq!(
                plan.skips[0].session, "/repo/atlas/.domux/worktrees/feature-x",
                "name {name:?}"
            );
        }
    }

    #[test]
    fn a_session_with_neither_a_name_nor_a_root_still_says_which_it_was() {
        let plan = planned(&[V1Session::default()]);
        assert_eq!(plan.skips[0].session, "a session with no name");
        assert_eq!(plan.skips[0].reason, "no root recorded");
    }

    #[test]
    fn the_report_line_counts_one_of_a_thing_without_an_s() {
        assert_eq!(
            report_line(Run::Real, 1, 1, 1),
            "Imported 1 project, 1 workspace, 1 tab."
        );
        assert_eq!(
            report_line(Run::Real, 3, 7, 12),
            "Imported 3 projects, 7 workspaces, 12 tabs."
        );
        assert_eq!(
            report_line(Run::Real, 0, 0, 0),
            "Imported 0 projects, 0 workspaces, 0 tabs."
        );
    }

    /// A dry run's summary says what would happen, in the summary itself. A note on another
    /// stream does not travel with it: `import v1 --dry-run > plan.txt` keeps this line and
    /// throws the note away, and "Imported 3 tabs" would then be a written record of work
    /// that never happened.
    #[test]
    fn a_dry_runs_summary_is_about_what_would_happen() {
        assert_eq!(
            report_line(Run::Dry, 1, 2, 3),
            "Would import 1 project, 2 workspaces, 3 tabs."
        );
        // The counts and their plurals are the same either way, so a reader can hold the
        // two summaries side by side and see one word differ.
        let real = report_line(Run::Real, 1, 2, 3);
        assert_eq!(
            real.trim_start_matches("Imported"),
            report_line(Run::Dry, 1, 2, 3).trim_start_matches("Would import")
        );
    }

    #[test]
    fn nothing_lost_is_no_failure_line() {
        assert_eq!(failure_line(0, 0, 0), None);
    }

    /// Each count keeps its own noun. A project can hold several sessions, so adding the
    /// counts and calling the total sessions states a number about the wrong thing: with
    /// two unreadable files and three refused projects the old line read "5 of V1's
    /// sessions did not come across", and five sessions is a number nothing measured.
    #[test]
    fn the_failure_line_names_the_noun_it_counts() {
        assert_eq!(
            failure_line(0, 3, 0).unwrap(),
            "Not imported: 3 projects. Read the messages above, then run this again."
        );
        assert_eq!(
            failure_line(2, 0, 0).unwrap(),
            "Not imported: 2 session files that would not read. Read the messages above, \
             then run this again."
        );
        assert_eq!(
            failure_line(0, 0, 1).unwrap(),
            "Not imported: 1 workspace. Read the messages above, then run this again."
        );
    }

    /// The three counts are distinct, so the fixture gives each a different value: a line
    /// built from the wrong field, or from their sum, cannot produce this string.
    #[test]
    fn the_failure_line_reports_all_three_kinds_at_once() {
        assert_eq!(
            failure_line(1, 2, 3).unwrap(),
            "Not imported: 1 session file that would not read, 2 projects, 3 workspaces. \
             Read the messages above, then run this again."
        );
    }

    #[test]
    fn counts_adds_up_every_workspace_and_every_tab() {
        let plan = planned(&[audrey_app(), workspace_1(), nameless()]);
        // Two projects; three workspaces, two of them in the first project; five tabs,
        // 1 and 2 in the first project and 2 in the second. No pair of these numbers is
        // equal, so a count that read only the first project or only the first workspace
        // cannot pass.
        assert_eq!(counts(&plan.projects), (2, 3, 5));
    }

    #[test]
    fn nothing_skipped_is_no_line_at_all() {
        assert_eq!(skipped_line(&[]), None);
    }

    #[test]
    fn the_skipped_line_names_each_session_and_its_reason() {
        let skips = vec![
            Skip {
                session: "audrey".into(),
                reason: "root is gone".into(),
            },
            Skip {
                session: "planner".into(),
                reason: "no root recorded".into(),
            },
        ];
        assert_eq!(
            skipped_line(&skips).unwrap(),
            "Skipped 2: audrey (root is gone), planner (no root recorded)."
        );
    }

    #[test]
    fn the_plan_names_every_project_workspace_name_and_tab() {
        let plan = planned(&[audrey_app(), workspace_1(), nameless()]);
        assert_eq!(
            plan_lines(&plan.projects),
            vec![
                "/repo/audrey-app",
                "  main, named audit-harness, tabs: agent-harness",
                "  workspace-1, named reviewer, tabs: comments, server",
                "/repo/planner",
                "  main, tabs: (unnamed), notes",
            ]
        );
    }

    #[test]
    fn a_workspace_with_no_tabs_says_only_what_it_is() {
        let planned = vec![PlannedProject {
            root: PathBuf::from("/repo/ledger"),
            workspaces: vec![PlannedWorkspace {
                handle: "main".into(),
                path: PathBuf::from("/repo/ledger"),
                name: None,
                tabs: Vec::new(),
            }],
        }];
        assert_eq!(plan_lines(&planned), vec!["/repo/ledger", "  main"]);
    }

    #[test]
    fn no_message_here_uses_an_em_dash() {
        let plan = planned(&[
            audrey_app(),
            workspace_1(),
            no_root(),
            handle_label(),
            not_a_slot_session(),
            duplicate(),
        ]);
        let mut text = plan_lines(&plan.projects).join("\n");
        text.push_str(&report_line(Run::Real, 1, 2, 3));
        text.push_str(&report_line(Run::Dry, 1, 2, 3));
        text.push_str(&failure_line(1, 2, 3).expect("three kinds"));
        text.push_str(&skipped_line(&plan.skips).expect("this set skips"));
        assert!(!text.contains('\u{2014}'), "{text}");
    }
}
