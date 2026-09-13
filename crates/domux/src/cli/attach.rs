//! The bare command and `attach`: start the server when needed, then attach.

use super::{call, call_as, socket};
use domux_client::{attach, control, AttachOutcome};
use domux_core::api::{ProjectAdded, ProjectInfo, WorkspaceInfo};
use domux_core::names::BIN_NAME;
use domux_core::shell;
use serde_json::json;
use std::ffi::OsString;
use std::fmt;
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
        // The offer below still runs. The server this starts seeds the directory it was
        // started in only when its state file holds no project, so a server started over a
        // state file with projects in it can be missing this directory like any running one.
        super::server::start(super::server::Announce::No).await?;
    }
    // The attach is what was asked for and the offer is an extra, so an offer that stops
    // short says why in one line and the attach goes on.
    if let Err(stopped) = offer_to_register_here().await {
        eprintln!("{stopped}");
    }
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
/// Everything here is best effort, because the attach is what was asked for and the offer is
/// an extra. Nobody to answer is not a failure: a directory that is gone, or a reader who is
/// not on a terminal, skips the offer and says nothing. Anything that stops the offer once
/// there is someone to ask comes back as `Stopped`, which `run` prints as one line before it
/// attaches: a server that will not answer `project.list`, a `project.add` it refuses, a
/// switch that fails. A git that will not answer only means the directory is treated as a
/// plain folder.
async fn offer_to_register_here() -> Result<(), Stopped> {
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(());
    };
    // Not a terminal means nothing to ask and nobody to answer. A script that pipes into
    // the client must not stop on a question it cannot see.
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(());
    }
    // Every side of the comparison is resolved, because the server answers with the path it
    // canonicalized and this reads the path the shell is standing in: on a Mac `/tmp/x` here
    // is `/private/tmp/x` there, and comparing them as written would offer to register a
    // directory that is already a project.
    let cwd = resolved(&cwd);
    let repository = repository_of(&cwd);
    let dir = repository
        .as_ref()
        .map_or_else(|| cwd.clone(), |r| r.top.clone());
    let asking = |cause: anyhow::Error| Stopped::Asking(dir.clone(), cause);
    let projects: Vec<ProjectInfo> = call_as("project.list", json!({})).await.map_err(asking)?;
    let workspaces: Vec<WorkspaceInfo> =
        call_as("workspace.list", json!({})).await.map_err(asking)?;
    // Only a repository can share a common directory, so outside one git is not asked.
    let common_dir = |root: &Path| repository.as_ref().and_then(|_| common_dir_of(root));
    let claims: Vec<Claim> = claims(&projects, &workspaces, common_dir)
        .into_iter()
        .map(|claim| Claim {
            path: resolved(&claim.path),
            ..claim
        })
        .collect();
    if at_home(&cwd, repository.as_ref(), &claims) {
        return Ok(());
    }
    // Checked before the question rather than after the yes: a question whose only answer
    // that does anything is certain to fail is not worth asking.
    let Some(path) = dir.to_str() else {
        return Err(not_utf8(dir));
    };
    if !answered_yes(&question(&dir)).map_err(|e| asking(e.into()))? {
        eprintln!("{}", declined(path));
        return Ok(());
    }
    // The same two calls `open` makes, in the same order and for the same reason: the
    // registration is true whether or not the switch works, so it is said first.
    let added: ProjectAdded = call_as("project.add", json!({ "path": path }))
        .await
        .map_err(|cause| Stopped::Registering(dir.clone(), cause))?;
    if !added.adopted.is_empty() {
        eprintln!("Adopted {}.", added.adopted.join(", "));
    }
    call("workspace.focus", json!({ "workspace": added.workspace }))
        .await
        .map_err(|cause| Stopped::Switching(dir.clone(), cause))?;
    Ok(())
}

/// Why the offer stopped short, with the directory it was about and the error that stopped it.
/// Its `Display` is the one line the reader is told before the attach goes on: what did not
/// happen, why, and the command that does it (principle 9).
#[derive(Debug)]
enum Stopped {
    /// Nothing was asked: the server's records would not read, or the terminal would not.
    Asking(PathBuf, anyhow::Error),
    /// The answer was yes and `project.add` failed, so nothing was registered.
    Registering(PathBuf, anyhow::Error),
    /// The directory was registered and `workspace.focus` failed, so the client lands where
    /// it was.
    Switching(PathBuf, anyhow::Error),
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Stopped::Asking(dir, cause)
        | Stopped::Registering(dir, cause)
        | Stopped::Switching(dir, cause)) = self;
        let shown = dir.display();
        let cause = one_line(cause);
        match self {
            Stopped::Asking(..) => {
                write!(f, "Could not ask whether to register {shown}: {cause}.")?
            }
            Stopped::Registering(..) => write!(f, "Could not register {shown}: {cause}.")?,
            Stopped::Switching(..) => {
                write!(f, "Registered {shown} but could not switch to it: {cause}.")?
            }
        }
        // A path that is not UTF-8 cannot be sent, so there is no command that would do it.
        let Some(path) = dir.to_str() else {
            return Ok(());
        };
        let open = format!("{BIN_NAME} open {}", shell::word(path));
        match self {
            Stopped::Asking(..) => write!(f, " Run {open} to register it."),
            Stopped::Registering(..) => write!(f, " Run {open} to try again."),
            Stopped::Switching(..) => write!(f, " Run {open} to switch to it."),
        }
    }
}

/// A directory whose path is not UTF-8. A path reaches the server as a JSON string, so there
/// is nothing the offer could send for it.
fn not_utf8(dir: PathBuf) -> Stopped {
    Stopped::Asking(
        dir,
        anyhow::anyhow!("its path is not valid UTF-8, which a project's path has to be"),
    )
}

/// An error and its causes as one line with no closing stop, so it can sit inside a sentence.
fn one_line(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    let words: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    words.join(" ").trim_end_matches('.').to_string()
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
/// would register the subdirectory. The line is for pasting, so a directory the shell would
/// split or expand is quoted.
fn declined(dir: &str) -> String {
    format!(
        "Left unregistered. Run {BIN_NAME} open {} to register it later.",
        shell::word(dir)
    )
}

/// The repository a directory is in.
#[derive(Debug)]
struct Repository {
    /// Its top level, which is what the offer registers.
    top: PathBuf,
    /// The git directory every work tree of the repository shares, when git says. A linked
    /// worktree has a top level of its own and shares this with its checkout.
    common_dir: Option<PathBuf>,
}

/// The repository `dir` is in, or `None` when it is in none or git will not say.
///
/// A fork, and it is allowed here because this is the command a person typed, before the
/// attach and nowhere near the core task. `rev-parse` answers in milliseconds.
fn repository_of(dir: &Path) -> Option<Repository> {
    // Read as a path and not as text, so a top level whose name is not UTF-8 is still the
    // directory it names, and the offer can say it cannot be registered.
    let top = domux_server::git::run_for_path(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .filter(|top| !top.as_os_str().is_empty())
        .map(|top| resolved(&top))?;
    let common_dir = common_dir_of(&top);
    Some(Repository { top, common_dir })
}

/// The common git directory of the work tree at `top`, resolved, or `None` when git will not
/// say.
///
/// Asked at a top level, where a relative answer means the same thing in every git version.
/// `--path-format=absolute` is git 2.31 and later; an older git does not know it, so anything
/// but one full path is asked again without it, and the relative answer is read from `top`.
fn common_dir_of(top: &Path) -> Option<PathBuf> {
    let git = |args: &[&str]| domux_server::git::run_for_path(top, args).ok();
    let common_dir = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .and_then(full_path_answer)
        .or_else(|| git(&["rev-parse", "--git-common-dir"]).map(|answer| top.join(answer)))?;
    common_dir.canonicalize().ok()
}

/// `answer` as a path when it is exactly one full path. An older git echoes the flag it does
/// not know on a line of its own before its answer, and that is not one.
fn full_path_answer(answer: PathBuf) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let one_line = !answer.as_os_str().as_bytes().contains(&b'\n');
    (answer.is_absolute() && one_line).then_some(answer)
}

/// A path the server holds: a project's root or a workspace's path.
struct Claim {
    path: PathBuf,
    /// Whether the claim belongs to a git project. A git project holds everything under its
    /// root, repositories included, so a slot, a worktree made by hand beside the slots and a
    /// submodule are all quiet. A folder project holds only the plain folders under it.
    git: bool,
    /// The common git directory of a git project's root, when git said. A work tree that
    /// shares it is the same repository wherever it was made. Only a root carries one: a slot
    /// shares its project's.
    common_dir: Option<PathBuf>,
}

/// Every path the server holds, each with the kind of the project it belongs to. `common_dir`
/// is asked about each git project's root and nothing else.
fn claims(
    projects: &[ProjectInfo],
    workspaces: &[WorkspaceInfo],
    common_dir: impl Fn(&Path) -> Option<PathBuf>,
) -> Vec<Claim> {
    let is_git = |kind: &str| kind == "git";
    let roots = projects.iter().map(|p| Claim {
        path: p.root.clone(),
        git: is_git(&p.kind),
        common_dir: if is_git(&p.kind) {
            common_dir(&p.root)
        } else {
            None
        },
    });
    let paths = workspaces.iter().map(|w| Claim {
        path: w.path.clone(),
        git: projects
            .iter()
            .any(|p| p.id == w.project && is_git(&p.kind)),
        common_dir: None,
    });
    roots.chain(paths).collect()
}

/// Whether `cwd` is already somewhere the server holds, so there is nothing to offer.
///
/// `repository` is the repository `cwd` is in, if it is in one. A git claim holds `cwd` when
/// `cwd` is the claim's path or lies under it, or when the repository shares the claim's common
/// git directory: a linked worktree is its project's wherever it was made. A folder claim holds
/// `cwd` on the path's terms only, and there must be no repository between the two: a
/// repository whose top level lies below the folder is a project of its own. A folder claim at
/// the top level or inside the repository still holds: a state file written before decision
/// record 0010 has folder records at repository roots, and those are the projects their readers
/// are in.
///
/// The common directory only ever adds to what is held. A submodule or a clone under a git
/// project has a common directory of its own, and the project's root still holds it, as it did
/// before (decision record 0041).
///
/// Pure: the caller resolves every path, `cwd`, the repository's and the claims', so that two
/// spellings of one directory compare equal.
fn at_home(cwd: &Path, repository: Option<&Repository>, claims: &[Claim]) -> bool {
    let common_dir = repository.and_then(|r| r.common_dir.as_ref());
    claims.iter().any(|claim| {
        if claim.git && common_dir.is_some() && claim.common_dir.as_ref() == common_dir {
            return true;
        }
        if !cwd.starts_with(&claim.path) {
            return false;
        }
        match repository {
            Some(r) if !claim.git => claim.path.starts_with(&r.top),
            _ => true,
        }
    })
}

/// `path` with its links resolved, or as it stands when it will not resolve.
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Asks the question and reads one line. Anything but yes, end of input included, is no: the
/// default in the prompt is the answer that changes nothing (principle 10).
fn answered_yes(question: &str) -> std::io::Result<bool> {
    let mut err = std::io::stderr();
    write!(err, "{question}")?;
    err.flush()?;
    read_yes(&mut std::io::stdin().lock())
}

/// Reads one line and answers whether it said yes. The line is read as bytes, so a line that
/// is not UTF-8 is not yes rather than a failure.
fn read_yes(input: &mut impl BufRead) -> std::io::Result<bool> {
    let mut line = Vec::new();
    if input.read_until(b'\n', &mut line)? == 0 {
        eprintln!();
        return Ok(false);
    }
    Ok(says_yes(&String::from_utf8_lossy(&line)))
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
    use std::os::unix::ffi::OsStringExt;

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

    /// A line that is not UTF-8 is not yes, and not a failure either: before, reading it as
    /// text failed, and the `?` on that read ended the attach.
    #[test]
    fn a_typed_line_that_is_not_utf8_is_read_as_no() {
        let mut typed: &[u8] = b"y\xe9\n";
        assert!(!read_yes(&mut typed).unwrap());
        let mut typed: &[u8] = b"yes\n";
        assert!(read_yes(&mut typed).unwrap());
        let mut nothing: &[u8] = b"";
        assert!(!read_yes(&mut nothing).unwrap());
    }

    fn git(path: &str) -> Claim {
        Claim {
            path: PathBuf::from(path),
            git: true,
            common_dir: None,
        }
    }

    /// A git project's root, with the common git directory git answered for it.
    fn git_sharing(path: &str, common_dir: &str) -> Claim {
        Claim {
            common_dir: Some(PathBuf::from(common_dir)),
            ..git(path)
        }
    }

    fn folder(path: &str) -> Claim {
        Claim {
            path: PathBuf::from(path),
            git: false,
            common_dir: None,
        }
    }

    /// A repository whose common git directory git did not say.
    fn repo(top: &str) -> Repository {
        Repository {
            top: PathBuf::from(top),
            common_dir: None,
        }
    }

    fn repo_sharing(top: &str, common_dir: &str) -> Repository {
        Repository {
            common_dir: Some(PathBuf::from(common_dir)),
            ..repo(top)
        }
    }

    const APP: &str = "/repo/audrey-app";

    #[test]
    fn a_directory_under_a_registered_path_is_already_at_home() {
        let claims = [git(APP), folder("/notes")];
        assert!(at_home(Path::new(APP), Some(&repo(APP)), &claims));
        assert!(at_home(
            Path::new("/repo/audrey-app/crates/api"),
            Some(&repo(APP)),
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
        let top = repo("/home/u/domux");
        assert!(!at_home(Path::new("/home/u/domux"), Some(&top), &claims));
        assert!(!at_home(
            Path::new("/home/u/domux/crates"),
            Some(&top),
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
            Some(&repo(APP)),
            &claims
        ));
        assert!(at_home(Path::new(slot), Some(&repo(slot)), &claims));
        let by_hand = "/repo/audrey-app/.worktrees/mux-37";
        assert!(at_home(Path::new(by_hand), Some(&repo(by_hand)), &claims));
    }

    /// A linked worktree made outside the project's root shares the project's common git
    /// directory, which is what makes it the same repository. No registered path holds it,
    /// and before this the offer asked about it on every attach.
    #[test]
    fn a_linked_worktree_of_a_git_project_is_at_home_wherever_it_was_made() {
        let common = "/repo/audrey-app/.git";
        let claims = [git_sharing(APP, common)];
        let feature = repo_sharing("/repo/app-feature", common);
        assert!(at_home(
            Path::new("/repo/app-feature"),
            Some(&feature),
            &claims
        ));
        assert!(at_home(
            Path::new("/repo/app-feature/crates"),
            Some(&feature),
            &claims
        ));

        // Under a folder project as well: the home project does not hold a repository, and
        // the git project beside the worktree does.
        let claims = [
            folder("/home/u"),
            git_sharing("/home/u/app", "/home/u/app/.git"),
        ];
        let feature = repo_sharing("/home/u/app-feature", "/home/u/app/.git");
        assert!(at_home(
            Path::new("/home/u/app-feature"),
            Some(&feature),
            &claims
        ));
    }

    /// A second clone of the same remote is a repository of its own, with a common directory
    /// of its own, and is offered like any other.
    #[test]
    fn a_repository_with_a_common_directory_of_its_own_is_not_at_home() {
        let claims = [
            folder("/home/u"),
            git_sharing("/home/u/app", "/home/u/app/.git"),
        ];
        let clone = repo_sharing("/home/u/app-2", "/home/u/app-2/.git");
        assert!(!at_home(Path::new("/home/u/app-2"), Some(&clone), &claims));
    }

    /// Only a git claim matches by common directory. `claims` never gives a folder one, and a
    /// folder record at a repository's root is not the repository's project either (decision
    /// record 0010).
    #[test]
    fn a_folder_claim_never_matches_by_common_directory() {
        let claims = [Claim {
            common_dir: Some(PathBuf::from("/repo/audrey-app/.git")),
            ..folder(APP)
        }];
        let feature = repo_sharing("/repo/app-feature", "/repo/audrey-app/.git");
        assert!(!at_home(
            Path::new("/repo/app-feature"),
            Some(&feature),
            &claims
        ));
    }

    /// The common directory only ever adds to what is at home. A submodule and a clone under a
    /// git project each have a common directory of their own, and they stay quiet as they
    /// were, because the project's root holds them (decision record 0041).
    #[test]
    fn a_submodule_or_a_clone_under_a_git_project_is_still_at_home() {
        let claims = [git_sharing(APP, "/repo/audrey-app/.git")];
        let submodule = repo_sharing(
            "/repo/audrey-app/vendor/lib",
            "/repo/audrey-app/.git/modules/lib",
        );
        assert!(at_home(
            Path::new("/repo/audrey-app/vendor/lib"),
            Some(&submodule),
            &claims
        ));
        let clone = repo_sharing(
            "/repo/audrey-app/fixtures/other",
            "/repo/audrey-app/fixtures/other/.git",
        );
        assert!(at_home(
            Path::new("/repo/audrey-app/fixtures/other"),
            Some(&clone),
            &claims
        ));
    }

    /// A folder record at a repository's top level is what a state file written before decision
    /// record 0010 holds, and a folder record inside a repository is a folder registered before
    /// the repository around it was made. Neither has a repository below it, so both hold where
    /// the reader is.
    #[test]
    fn a_folder_project_at_or_inside_the_repository_is_at_home() {
        assert!(at_home(
            Path::new("/repo/audrey-app/crates"),
            Some(&repo(APP)),
            &[folder(APP)]
        ));
        assert!(at_home(
            Path::new("/repo/audrey-app/docs/notes"),
            Some(&repo(APP)),
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
        assert!(!at_home(Path::new(APP), Some(&repo(APP)), &[]));
        assert!(!at_home(Path::new("/notes"), None, &[]));
    }

    /// A workspace is held on its project's terms, so a slot of a git project holds the
    /// repositories under it and the `main` of a folder project does not. Only a git
    /// project's root is asked for its common directory: a slot shares its project's, and a
    /// folder has none that counts.
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
        let asked = std::cell::RefCell::new(Vec::new());
        let said: Vec<(PathBuf, bool, Option<PathBuf>)> = claims(&projects, &workspaces, |root| {
            asked.borrow_mut().push(root.to_path_buf());
            Some(root.join(".git"))
        })
        .into_iter()
        .map(|c| (c.path, c.git, c.common_dir))
        .collect();
        assert_eq!(
            said,
            [
                (
                    PathBuf::from(APP),
                    true,
                    Some(PathBuf::from("/repo/audrey-app/.git"))
                ),
                (PathBuf::from("/home/u"), false, None),
                (PathBuf::from(slot), true, None),
                (PathBuf::from("/home/u"), false, None),
            ]
        );
        assert_eq!(asked.into_inner(), [PathBuf::from(APP)]);
    }

    /// `--path-format` arrived in git 2.31. An older git echoes a flag it does not know on a
    /// line of its own, so anything but one full path is asked again without it.
    #[test]
    fn only_one_full_path_is_taken_as_the_common_directory() {
        let answer = |said: &str| full_path_answer(PathBuf::from(said));
        assert_eq!(
            answer("/repo/audrey-app/.git"),
            Some(PathBuf::from("/repo/audrey-app/.git"))
        );
        assert_eq!(answer("--path-format=absolute\n.git"), None);
        assert_eq!(answer(".git"), None);
        assert_eq!(answer(""), None);
    }

    /// Against real repositories: the checkout and a worktree made beside it answer one common
    /// directory, a repository of its own answers another, and a plain folder answers none.
    #[test]
    fn every_work_tree_of_one_repository_answers_the_same_common_directory() {
        let (tmp, app) = domux_server::testing::repo_with_origin("main");
        let feature = tmp.path().join("app-feature");
        domux_server::testing::git(
            &app,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                feature.to_str().unwrap(),
            ],
        );
        let other = tmp.path().join("other");
        std::fs::create_dir(&other).unwrap();
        domux_server::testing::git(&other, &["init", "-q", "-b", "main"]);
        let plain = tmp.path().join("plain");
        std::fs::create_dir(&plain).unwrap();

        let common = common_dir_of(&app).expect("the checkout has a common directory");
        assert_eq!(common, app.join(".git").canonicalize().unwrap());
        let inside = feature.join("crates");
        std::fs::create_dir(&inside).unwrap();
        let found = repository_of(&inside).expect("the worktree is a repository");
        assert_eq!(found.top, feature.canonicalize().unwrap());
        assert_eq!(found.common_dir.as_ref(), Some(&common));
        assert_ne!(common_dir_of(&other), Some(common));
        assert_eq!(common_dir_of(&plain), None);
        assert!(repository_of(&plain).is_none());
    }

    /// A directory's name does not have to be UTF-8, and git prints a top level as the
    /// directory's own bytes. Read as text, `caf\xe9` became `caf\u{fffd}`, a directory that
    /// is not there, so the offer asked about a path that could not be registered and then
    /// said it did not exist.
    #[test]
    fn a_repository_whose_path_is_not_utf8_is_found_at_its_own_path() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join(OsString::from_vec(b"caf\xe9".to_vec()));
        // A file system that refuses a name that is not UTF-8, as APFS does, has no such
        // repository to find.
        if std::fs::create_dir(&repo).is_err() {
            return;
        }
        domux_server::testing::git(&repo, &["init", "-q", "-b", "main"]);
        let inside = repo.join("src");
        std::fs::create_dir(&inside).unwrap();

        let found = repository_of(&inside).expect("the directory is a repository");
        assert_eq!(found.top, repo.canonicalize().unwrap());
        assert_eq!(
            found.common_dir,
            Some(repo.join(".git").canonicalize().unwrap())
        );
    }

    /// `open .` registers the directory it is typed in, which is the wrong one when the offer
    /// was about the repository above it.
    #[test]
    fn the_decline_line_names_the_directory_to_open() {
        assert_eq!(
            declined(APP),
            "Left unregistered. Run domux open /repo/audrey-app to register it later."
        );
    }

    /// The line is for pasting, so a directory a shell would split or expand is quoted.
    #[test]
    fn the_decline_line_quotes_a_directory_the_shell_would_split() {
        assert_eq!(
            declined("/home/u/my notes"),
            "Left unregistered. Run domux open '/home/u/my notes' to register it later."
        );
    }

    /// What is pasted is what was asked about: the command in the line, run by a shell,
    /// hands `open` the directory back whole.
    #[test]
    fn the_command_in_the_decline_line_hands_a_shell_the_directory_whole() {
        for dir in ["/home/u/my notes", "/home/u/it's $HOME", "/home/u/a*b [1]"] {
            let line = declined(dir);
            let command = line
                .strip_prefix("Left unregistered. Run domux open ")
                .and_then(|rest| rest.strip_suffix(" to register it later."))
                .unwrap_or_else(|| panic!("{line}"));
            let out = std::process::Command::new("sh")
                .args(["-c", &format!("printf %s {command}")])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), dir, "{line}");
        }
    }

    /// The offer is best effort, and whatever stops it short is one line: what did not
    /// happen, why, and the command that does it, before the attach goes on.
    #[test]
    fn a_failed_offer_says_what_did_not_happen_why_and_what_to_run() {
        let dir = || PathBuf::from("/home/u/my app");
        let unanswered = anyhow::anyhow!("early eof")
            .context("The server did not answer project.list on /s. Run domux server status.");
        assert_eq!(
            Stopped::Asking(dir(), unanswered).to_string(),
            "Could not ask whether to register /home/u/my app: The server did not answer project.list on /s. Run domux server status.: early eof. Run domux open '/home/u/my app' to register it."
        );
        assert_eq!(
            Stopped::Registering(
                dir(),
                anyhow::anyhow!("not_found: /home/u/my app does not exist")
            )
            .to_string(),
            "Could not register /home/u/my app: not_found: /home/u/my app does not exist. Run domux open '/home/u/my app' to try again."
        );
        assert_eq!(
            Stopped::Switching(dir(), anyhow::anyhow!("not_found: no workspace w_0002.")).to_string(),
            "Registered /home/u/my app but could not switch to it: not_found: no workspace w_0002. Run domux open '/home/u/my app' to switch to it."
        );
    }

    #[test]
    fn a_failed_offer_is_one_line_whatever_the_cause_holds() {
        let said = Stopped::Registering(
            PathBuf::from("/repo"),
            anyhow::anyhow!("internal: first\nsecond\n"),
        )
        .to_string();
        assert_eq!(
            said,
            "Could not register /repo: internal: first second. Run domux open /repo to try again."
        );
    }

    /// A path reaches the server as a JSON string, so a directory whose name is not UTF-8
    /// cannot be registered and is not asked about. There is no command to name for it.
    #[test]
    fn a_directory_that_is_not_utf8_is_not_asked_about_and_names_no_command() {
        let dir = PathBuf::from(std::ffi::OsString::from_vec(b"/home/u/caf\xe9".to_vec()));
        assert_eq!(
            not_utf8(dir).to_string(),
            "Could not ask whether to register /home/u/caf\u{fffd}: its path is not valid UTF-8, which a project's path has to be."
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
