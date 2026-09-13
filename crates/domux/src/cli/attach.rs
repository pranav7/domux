//! The bare command and `attach`: start the server when needed, then attach.

use super::{call, call_as, socket};
use domux_client::{attach, control, AttachOutcome};
use domux_core::api::{ProjectAdded, ProjectInfo, WorkspaceInfo};
use domux_core::names::BIN_NAME;
use serde_json::json;
use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// The bare command, which is how the attach is normally typed. Inside a pane it refuses: a
/// second whole screen drawn inside one pane of the screen it is drawing takes the keys from
/// the outer client (principle 1) and puts a second accent border on the screen (principle
/// 2). Nobody types it there wanting that; they type it out of habit.
///
/// `attach` typed in full still attaches, and that is deliberate. The bare command is the
/// habit; naming the verb is a choice. It is also the only way to reach a different server from
/// inside a pane: `DOMUX_SOCKET` set by hand points at that server, but `DOMUX_PANE` is still
/// the outer pane's, so a guard on both forms would refuse a thing worth doing.
///
/// Every other subcommand is unaffected. `DOMUX_SOCKET` is exported into every pane so a shell
/// there can reach its own server, which is the whole reason the variable exists.
pub async fn run_bare() -> anyhow::Result<()> {
    let socket = socket();
    if inside_a_pane(std::env::var_os("DOMUX_PANE")) && control::is_live(&socket).await {
        return Err(nested_attach());
    }
    run().await
}

pub async fn run() -> anyhow::Result<()> {
    let socket = socket();
    if !control::is_live(&socket).await {
        // The attach that follows is the answer, so the start says nothing: telling the
        // reader to attach would name an action already underway.
        //
        // The server this starts seeds the directory it was started in, so the offer below
        // has nothing to offer: the question is only ever asked of a server that was already
        // running somewhere else.
        super::server::start(super::server::Announce::No).await?;
    }
    offer_to_register_here().await?;
    // `attach` returns with the terminal already restored, so the line below lands on a
    // terminal the reader can type into again (principle 11).
    eprintln!("{}", ending(attach(&socket).await?)?);
    Ok(())
}

/// Asks, once, whether the directory this was typed in should become a project, and registers
/// it when the answer is yes.
///
/// Decision record 0006 settled that attach reconnects to what is registered rather than
/// following the shell, and that still holds: this does not move the client to the directory
/// on its own. What it adds is the offer, because the old behaviour had no way to say yes.
/// Typing the bare command somewhere new landed you in another project's tabs with nothing
/// on the screen about the directory you were standing in, and the only way out was to know
/// that `open` existed.
///
/// It asks rather than registering, so a bare command typed in a scratch directory, a
/// downloads folder or somebody else's checkout does not quietly leave a project behind.
///
/// The directory it asks about is the repository's top level when this was typed inside a
/// repository, and the directory itself otherwise, because that is what a project is. It says
/// nothing when `at_home` finds the directory already held, and a folder project holds only
/// the plain folders under it, not the repositories (decision record 0041). Before that, a
/// server first started in the home directory seeded it as a folder project, and every
/// repository under it was held and never offered.
///
/// Everything here is best effort. A directory that cannot be read, a server that will not
/// answer `project.list`, a reader who is not on a terminal: each one skips the offer and
/// attaches, because the attach is what was asked for and the offer is an extra. A git that
/// will not answer only means the directory is treated as a plain folder.
async fn offer_to_register_here() -> anyhow::Result<()> {
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(());
    };
    // Not a terminal means nothing to ask and nobody to answer. A script that pipes into
    // the client must not stop on a question it cannot see.
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(());
    }
    let projects: Vec<ProjectInfo> = call_as("project.list", json!({})).await?;
    let workspaces: Vec<WorkspaceInfo> = call_as("workspace.list", json!({})).await?;
    let repository = repository_of(&cwd);
    if at_home(&cwd, repository.as_deref(), &claims(&projects, &workspaces)) {
        return Ok(());
    }
    let dir = repository.unwrap_or(cwd);
    if !answered_yes(&question(&dir))? {
        eprintln!("{}", declined(&dir));
        return Ok(());
    }
    // The same two calls `open` makes, in the same order and for the same reason: the
    // registration is true whether or not the switch works, so it is said first.
    let added: ProjectAdded = call_as("project.add", json!({ "path": dir })).await?;
    if !added.adopted.is_empty() {
        eprintln!("Adopted {}.", added.adopted.join(", "));
    }
    call("workspace.focus", json!({ "workspace": added.workspace })).await?;
    Ok(())
}

/// The question, as the state, the object and the next action (principle 9).
fn question(dir: &Path) -> String {
    format!(
        "{} is not a project yet. Register it? [y/N] ",
        dir.display()
    )
}

/// What a reader who said no is told. It names the directory rather than `.`, because the
/// directory asked about is the repository's top level and `open .` typed in a subdirectory
/// would register the subdirectory.
fn declined(dir: &Path) -> String {
    format!(
        "Left unregistered. Run {BIN_NAME} open {} to register it later.",
        dir.display()
    )
}

/// The top level of the repository `dir` is in, or `None` when it is in none or git will not
/// say.
///
/// A fork, and it is allowed here because this is the command a person typed, before the
/// attach and nowhere near the core task. `rev-parse` answers in milliseconds.
fn repository_of(dir: &Path) -> Option<PathBuf> {
    domux_server::git::run(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .filter(|top| !top.is_empty())
        .map(PathBuf::from)
}

/// A path the server holds: a project's root or a workspace's path.
struct Claim {
    path: PathBuf,
    /// Whether the claim belongs to a git project. A git project holds everything under its
    /// root, repositories included, so a slot, a worktree made by hand beside the slots and a
    /// submodule are all quiet. A folder project holds only the plain folders under it.
    git: bool,
}

/// Every path the server holds, each with the kind of the project it belongs to.
fn claims(projects: &[ProjectInfo], workspaces: &[WorkspaceInfo]) -> Vec<Claim> {
    let is_git = |kind: &str| kind == "git";
    let roots = projects.iter().map(|p| Claim {
        path: p.root.clone(),
        git: is_git(&p.kind),
    });
    let paths = workspaces.iter().map(|w| Claim {
        path: w.path.clone(),
        git: projects
            .iter()
            .any(|p| p.id == w.project && is_git(&p.kind)),
    });
    roots.chain(paths).collect()
}

/// Whether `cwd` is already somewhere the server holds, so there is nothing to offer.
///
/// `repository` is the top level of the repository `cwd` is in, if it is in one. A git claim
/// holds `cwd` when `cwd` is the claim's path or lies under it. A folder claim holds it on the
/// same terms, except that there must be no repository between the two: a repository whose top
/// level lies below the folder is a project of its own. A folder claim at the top level or
/// inside the repository still holds: a state file written before decision record 0010 has
/// folder records at repository roots, and those are the projects their readers are in.
///
/// Every side is resolved, because the server answers with the path it canonicalized and
/// this reads the path the shell is standing in: on a Mac `/tmp/x` here is `/private/tmp/x`
/// there, and comparing them as written would offer to register a directory that is already
/// a project. A path that will not resolve is compared as it stands.
fn at_home(cwd: &Path, repository: Option<&Path>, claims: &[Claim]) -> bool {
    let cwd = resolved(cwd);
    let repository = repository.map(resolved);
    claims.iter().any(|claim| {
        let path = resolved(&claim.path);
        if !cwd.starts_with(&path) {
            return false;
        }
        match &repository {
            Some(top) if !claim.git => path.starts_with(top),
            _ => true,
        }
    })
}

fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Reads one line and answers whether it said yes. Anything else, end of input included, is
/// no: the default in the prompt is the answer that changes nothing (principle 10).
fn answered_yes(question: &str) -> anyhow::Result<bool> {
    let mut err = std::io::stderr();
    write!(err, "{question}")?;
    err.flush()?;
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        eprintln!();
        return Ok(false);
    }
    Ok(says_yes(&line))
}

/// Whether one typed line said yes.
fn says_yes(line: &str) -> bool {
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

/// What the reader is told when the attach ends. The two outcomes that are not a failure
/// answer with the line to print, the two that are answer with the error to report, so a
/// refused attach cannot end in silence with a status of 0.
fn ending(outcome: AttachOutcome) -> anyhow::Result<String> {
    match outcome {
        AttachOutcome::Detached(_) => Ok(format!("Detached. Run {BIN_NAME} to reattach.")),
        AttachOutcome::ServerStopped => Ok("The server stopped.".to_string()),
        AttachOutcome::ConnectionLost => Err(anyhow::anyhow!(
            "Lost the connection to the server. Run {BIN_NAME} server status."
        )),
        AttachOutcome::Refused(reason) => Err(anyhow::anyhow!("{reason}")),
    }
}

/// The state, the object and the next action (principle 9), in the shape the tmux refusal in
/// `domux-client` already uses.
fn nested_attach() -> anyhow::Error {
    anyhow::anyhow!(
        "{BIN_NAME} is already running in this terminal. Its panes are this one's screen; run {BIN_NAME} tab create or another subcommand here, or open a new terminal to attach a second view."
    )
}

/// Whether this shell is already inside a pane.
///
/// `DOMUX_PANE` and not `DOMUX_SOCKET`, though the server exports both. `DOMUX_SOCKET` is also
/// a documented override - it is how you point the CLI at a scratch server beside the real one,
/// and how the tests reach a harness - so a shell that exports it is not in a pane, and
/// refusing there would tell the reader they are somewhere they are not while leaving them no
/// way to attach at all. `DOMUX_PANE` carries a pane id and the server is the only thing that
/// sets it, so it means what this asks. An `ssh` to another machine carries neither.
fn inside_a_pane(pane_var: Option<OsString>) -> bool {
    pane_var.is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::ids::{ProjectId, WorkspaceId};

    #[test]
    fn a_detach_and_a_stopped_server_are_endings_the_reader_is_told_about() {
        assert_eq!(
            ending(AttachOutcome::Detached("detached".into())).unwrap(),
            "Detached. Run domux to reattach."
        );
        assert_eq!(
            ending(AttachOutcome::ServerStopped).unwrap(),
            "The server stopped."
        );
    }

    #[test]
    fn a_lost_connection_and_a_refusal_are_failures_with_a_next_action() {
        let lost = ending(AttachOutcome::ConnectionLost)
            .unwrap_err()
            .to_string();
        assert_eq!(
            lost,
            "Lost the connection to the server. Run domux server status."
        );
        // The server's own sentence, which already names both versions and the way out.
        let refused = ending(AttachOutcome::Refused(
            "the server is domux 2.0.0 and this client is 1.9.0".into(),
        ))
        .unwrap_err()
        .to_string();
        assert_eq!(
            refused,
            "the server is domux 2.0.0 and this client is 1.9.0"
        );
    }

    #[test]
    fn only_yes_registers_and_everything_else_leaves_the_directory_alone() {
        assert!(says_yes("y"));
        assert!(says_yes("Y\n"));
        assert!(says_yes("  yes  "));
        assert!(!says_yes(""));
        assert!(!says_yes("n"));
        assert!(!says_yes("yeah"));
        assert!(!says_yes("yes please"));
    }

    fn git(path: &str) -> Claim {
        Claim {
            path: PathBuf::from(path),
            git: true,
        }
    }

    fn folder(path: &str) -> Claim {
        Claim {
            path: PathBuf::from(path),
            git: false,
        }
    }

    const APP: &str = "/repo/audrey-app";

    #[test]
    fn a_directory_under_a_registered_path_is_already_at_home() {
        let claims = [git(APP), folder("/notes")];
        assert!(at_home(Path::new(APP), Some(Path::new(APP)), &claims));
        assert!(at_home(
            Path::new("/repo/audrey-app/crates/api"),
            Some(Path::new(APP)),
            &claims
        ));
        assert!(at_home(Path::new("/notes"), None, &claims));
    }

    /// MUX-37. The start-up seed registered the home directory as a folder project, and every
    /// repository under it was then at home, so attach typed in one of them never asked. A
    /// folder project holds folders; a repository below it is a project of its own.
    #[test]
    fn a_repository_under_a_folder_project_is_not_at_home() {
        let claims = [folder("/home/u")];
        let top = Path::new("/home/u/domux");
        assert!(!at_home(top, Some(top), &claims));
        assert!(!at_home(
            Path::new("/home/u/domux/crates"),
            Some(top),
            &claims
        ));
    }

    #[test]
    fn a_plain_folder_under_a_folder_project_is_still_at_home() {
        let claims = [folder("/home/u")];
        assert!(at_home(Path::new("/home/u"), None, &claims));
        assert!(at_home(Path::new("/home/u/notes"), None, &claims));
    }

    /// A git project holds everything under its root, so a subdirectory, a slot and a linked
    /// worktree made by hand beside the slots are all quiet, although each of the last two is
    /// a work tree of its own with its own top level.
    #[test]
    fn a_directory_under_a_git_project_or_its_slot_is_still_at_home() {
        let slot = "/repo/audrey-app/.domux/worktrees/workspace-1";
        let claims = [git(APP), git(slot)];
        assert!(at_home(
            Path::new("/repo/audrey-app/crates"),
            Some(Path::new(APP)),
            &claims
        ));
        assert!(at_home(Path::new(slot), Some(Path::new(slot)), &claims));
        let by_hand = Path::new("/repo/audrey-app/.worktrees/mux-37");
        assert!(at_home(by_hand, Some(by_hand), &claims));
    }

    /// A folder record at a repository's top level is what a state file written before decision
    /// record 0010 holds, and a folder record inside a repository is a folder registered before
    /// the repository around it was made. Neither has a repository below it, so both hold where
    /// the reader is.
    #[test]
    fn a_folder_project_at_or_inside_the_repository_is_at_home() {
        assert!(at_home(
            Path::new("/repo/audrey-app/crates"),
            Some(Path::new(APP)),
            &[folder(APP)]
        ));
        assert!(at_home(
            Path::new("/repo/audrey-app/docs/notes"),
            Some(Path::new(APP)),
            &[folder("/repo/audrey-app/docs")]
        ));
    }

    /// Component by component, not character by character: `/repo/audrey-app-2` is a
    /// different directory from `/repo/audrey-app` and must still be offered.
    #[test]
    fn a_directory_whose_name_merely_starts_the_same_is_not_at_home() {
        let claims = [git(APP)];
        assert!(!at_home(Path::new("/repo/audrey-app-2"), None, &claims));
        assert!(!at_home(Path::new("/repo"), None, &claims));
        assert!(!at_home(Path::new("/elsewhere"), None, &claims));
    }

    #[test]
    fn nothing_registered_means_every_directory_is_offered() {
        assert!(!at_home(Path::new(APP), Some(Path::new(APP)), &[]));
        assert!(!at_home(Path::new("/notes"), None, &[]));
    }

    /// A workspace is held on its project's terms, so a slot of a git project holds the
    /// repositories under it and the `main` of a folder project does not.
    #[test]
    fn a_workspace_path_takes_the_kind_of_its_project() {
        let project = |id: &str, root: &str, kind: &str| ProjectInfo {
            id: ProjectId(id.into()),
            name: "x".into(),
            root: PathBuf::from(root),
            kind: kind.into(),
            default_branch: None,
            workspaces: 1,
        };
        let workspace = |project: &str, path: &str| WorkspaceInfo {
            id: WorkspaceId("w_0001".into()),
            project: ProjectId(project.into()),
            handle: "main".into(),
            name: None,
            path: PathBuf::from(path),
            branch: None,
            pr: None,
            pr_state: None,
            tabs: 1,
        };
        let projects = [
            project("pr_0001", APP, "git"),
            project("pr_0002", "/home/u", "folder"),
        ];
        let slot = "/elsewhere/audrey-app-1";
        let workspaces = [workspace("pr_0001", slot), workspace("pr_0002", "/home/u")];
        let said: Vec<(PathBuf, bool)> = claims(&projects, &workspaces)
            .into_iter()
            .map(|c| (c.path, c.git))
            .collect();
        assert_eq!(
            said,
            [
                (PathBuf::from(APP), true),
                (PathBuf::from("/home/u"), false),
                (PathBuf::from(slot), true),
                (PathBuf::from("/home/u"), false),
            ]
        );
    }

    /// `open .` registers the directory it is typed in, which is the wrong one when the offer
    /// was about the repository above it.
    #[test]
    fn the_decline_line_names_the_directory_to_open() {
        assert_eq!(
            declined(Path::new(APP)),
            "Left unregistered. Run domux open /repo/audrey-app to register it later."
        );
    }

    #[test]
    fn the_question_names_the_directory_and_defaults_to_leaving_it_alone() {
        let said = question(Path::new("/repo/audrey-app"));
        assert_eq!(
            said,
            "/repo/audrey-app is not a project yet. Register it? [y/N] "
        );
    }

    #[test]
    fn a_shell_is_inside_a_pane_only_when_the_pane_variable_has_a_value() {
        assert!(!inside_a_pane(None));
        assert!(!inside_a_pane(Some(OsString::new())));
        assert!(inside_a_pane(Some(OsString::from("p_0001"))));
    }

    /// The variable this reads has to be one only the server sets. `DOMUX_SOCKET` is also the
    /// documented way to point the CLI at a scratch server beside the real one, so reading it
    /// here refused every attach for anyone who exports it - and told them they were inside a
    /// pane when they were not, with no way to attach at all.
    #[test]
    fn the_guard_reads_the_pane_variable_and_not_the_socket_override() {
        let source = include_str!("attach.rs");
        let call = source
            .lines()
            .find(|l| l.contains("inside_a_pane(std::env::var_os("))
            .expect("the guard's call site");
        assert!(call.contains("DOMUX_PANE"), "{call}");
        assert!(!call.contains("DOMUX_SOCKET"), "{call}");
    }

    #[test]
    fn the_nested_attach_refusal_names_the_state_the_object_and_the_next_action() {
        let said = nested_attach().to_string();
        assert!(
            said.starts_with("domux is already running in this terminal."),
            "{said}"
        );
        assert!(said.contains("domux tab create"), "{said}");
        assert!(said.contains("open a new terminal"), "{said}");
    }
}
