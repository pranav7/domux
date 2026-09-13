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
/// attaches: a server that will not answer `project.list`, a terminal that will not take the
/// question, a `project.add` the server refuses, a switch that fails. A git that will not answer only means the directory is treated as a
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
    let top = top_level_of(&cwd);
    let dir = top.clone().unwrap_or_else(|| cwd.clone());
    let reading = |cause: anyhow::Error| Stopped::Reading(dir.clone(), cause);
    let projects: Vec<ProjectInfo> = call_as("project.list", json!({})).await.map_err(reading)?;
    let workspaces: Vec<WorkspaceInfo> = call_as("workspace.list", json!({}))
        .await
        .map_err(reading)?;
    let claims: Vec<Claim> = claims(&projects, &workspaces)
        .into_iter()
        .map(|claim| Claim {
            path: resolved(&claim.path),
            ..claim
        })
        .collect();
    if at_home(&cwd, top.as_deref(), &claims, common_dir_of) {
        return Ok(());
    }
    // Checked before the question rather than after the yes: a question whose only answer
    // that does anything is certain to fail is not worth asking.
    let Some(path) = dir.to_str() else {
        return Err(not_utf8(dir));
    };
    if !answered_yes(&question(&dir)).map_err(|e| Stopped::Asking(dir.clone(), e.into()))? {
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
    /// Nothing was asked: the server would not give its projects and workspaces.
    Reading(PathBuf, anyhow::Error),
    /// Nothing was asked: the terminal would not take the question or give the answer, or the
    /// path cannot be sent.
    Asking(PathBuf, anyhow::Error),
    /// The answer was yes and `project.add` failed, so nothing was registered.
    Registering(PathBuf, anyhow::Error),
    /// The directory was registered and `workspace.focus` failed, so the client lands where
    /// it was.
    Switching(PathBuf, anyhow::Error),
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Stopped::Reading(dir, cause)
        | Stopped::Asking(dir, cause)
        | Stopped::Registering(dir, cause)
        | Stopped::Switching(dir, cause)) = self;
        let shown = dir.display();
        let cause = one_line(cause);
        match self {
            Stopped::Reading(..) | Stopped::Asking(..) => {
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
            // The cause says what to do about the server, and `open` goes through that same
            // server, so it is not a second thing to try.
            Stopped::Reading(..) => Ok(()),
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
///
/// A cause is joined to the context above it with a colon, as `{:#}` joins them, except under a
/// context that is already whole sentences, such as the one that names `domux server status`.
/// That context goes after its causes, so the line ends on the action it names rather than
/// reading `status.: early eof`.
fn one_line(error: &anyhow::Error) -> String {
    let mut links = error.chain().rev().map(|link| {
        let text = link.to_string();
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        lines.join(" ")
    });
    let root = links.next().unwrap_or_default();
    links.fold(root.trim_end_matches('.').to_string(), |line, context| {
        if context.ends_with('.') {
            format!("{line}. {}", context.trim_end_matches('.'))
        } else {
            format!("{context}: {line}")
        }
    })
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

/// The top level of the repository `dir` is in, resolved, or `None` when it is in none or git
/// will not say.
///
/// A fork, and it is allowed here because this is the command a person typed, before the
/// attach and nowhere near the core task.
fn top_level_of(dir: &Path) -> Option<PathBuf> {
    // Read as a path and not as text, so a top level whose name is not UTF-8 is still the
    // directory it names, and the offer can say it cannot be registered.
    domux_server::git::run_for_path(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .filter(|top| !top.as_os_str().is_empty())
        .map(|top| resolved(&top))
}

/// The common directory of the worktree at `dir`, resolved, or `None` when git will not say.
///
/// The common directory is the git directory every worktree of a repository shares: `.git` in
/// the checkout, which a linked worktree's `.git` file points back to. Asking costs one git
/// process, or two on a git older than 2.31.
fn common_dir_of(dir: &Path) -> Option<PathBuf> {
    let git = |args: &[&str]| domux_server::git::run_for_path(dir, args).ok();
    let absolute = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    common_dir_from(dir, absolute, || git(&["rev-parse", "--git-common-dir"]))?
        .canonicalize()
        .ok()
}

/// The common directory in git's answers, asked in `dir`. `absolute` is the answer to the
/// question with `--path-format=absolute`, or `None` when git refused it, and `again` asks
/// without the flag.
///
/// `--path-format` is git 2.31 and later, and an older git does not refuse a flag it does not
/// know: it prints the flag back on a line of its own before its answer. So anything but one
/// full path is asked again without the flag, and a relative answer is read from `dir`, the
/// directory git was asked in. A refusal is not asked again: it means a directory that is gone
/// or is in no repository, and git would refuse the second question too.
fn common_dir_from(
    dir: &Path,
    absolute: Option<PathBuf>,
    again: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let answer = absolute?;
    if answer.is_absolute() && !answer.as_os_str().as_bytes().contains(&b'\n') {
        return Some(answer);
    }
    again().map(|relative| dir.join(relative))
}

/// A path the server holds: a project's root or a workspace's path.
struct Claim {
    path: PathBuf,
    /// Whether the claim belongs to a git project. A git project holds everything under its
    /// root, repositories included, so a slot, a worktree made by hand beside the slots and a
    /// submodule are all quiet. A folder project holds only the plain folders under it.
    git: bool,
    /// Whether the path is its project's root. Only a git project's root is asked for its
    /// common directory: a slot shares its project's.
    root: bool,
}

/// Every path the server holds, each with the kind of the project it belongs to.
fn claims(projects: &[ProjectInfo], workspaces: &[WorkspaceInfo]) -> Vec<Claim> {
    let is_git = |kind: &str| kind == "git";
    let roots = projects.iter().map(|p| Claim {
        path: p.root.clone(),
        git: is_git(&p.kind),
        root: true,
    });
    let paths = workspaces.iter().map(|w| Claim {
        path: w.path.clone(),
        git: projects
            .iter()
            .any(|p| p.id == w.project && is_git(&p.kind)),
        root: false,
    });
    roots.chain(paths).collect()
}

/// Whether `cwd` is already somewhere the server holds, so there is nothing to offer.
///
/// `top` is the top level of the repository `cwd` is in, if it is in one. A claim holds `cwd`
/// when `cwd` is the claim's path or lies under it. A folder claim holds it only when there is
/// no repository between the two: a repository whose top level lies below the folder is a
/// project of its own. A folder claim at the top level or inside the repository still holds: a
/// state file written before decision record 0010 has folder records at repository roots, and
/// those are the projects their readers are in.
///
/// When no path holds `cwd`, a git project's root still holds a repository whose common
/// directory is its own or lies inside it: a linked worktree shares the project's, and a
/// submodule checked out in a linked worktree keeps its own under the project's
/// `.git/worktrees`, so both are the project's wherever they were made (decision record 0041).
/// A git directory inside another repository's is only ever a worktree's or a submodule's.
/// `common_dir` answers the common directory of the worktree at a path, and every
/// answer is a git process, so it is asked only then: first about `top`, then about each git
/// project's root until one holds.
///
/// The common directory only ever adds to what is held. A submodule or a clone under a git
/// project has a common directory of its own, and the project's root holds it by its path, as
/// it did before.
///
/// The caller resolves every path, `cwd`, `top` and the claims', so that two spellings of one
/// directory compare equal.
fn at_home(
    cwd: &Path,
    top: Option<&Path>,
    claims: &[Claim],
    mut common_dir: impl FnMut(&Path) -> Option<PathBuf>,
) -> bool {
    let by_path = claims.iter().any(|claim| {
        cwd.starts_with(&claim.path)
            && (claim.git || top.is_none_or(|top| claim.path.starts_with(top)))
    });
    if by_path {
        return true;
    }
    let Some(shared) = top.and_then(&mut common_dir) else {
        return false;
    };
    claims
        .iter()
        .filter(|claim| claim.git && claim.root)
        .any(|claim| common_dir(&claim.path).is_some_and(|root| shared.starts_with(root)))
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

    /// A git project's root.
    fn git(path: &str) -> Claim {
        Claim {
            path: PathBuf::from(path),
            git: true,
            root: true,
        }
    }

    /// A workspace of a git project, whose path is not the project's root.
    fn slot(path: &str) -> Claim {
        Claim {
            root: false,
            ..git(path)
        }
    }

    fn folder(path: &str) -> Claim {
        Claim {
            path: PathBuf::from(path),
            git: false,
            root: true,
        }
    }

    /// What git answers for the common directory of each path, and every path it was asked
    /// about, in order. A path with no answer is in no repository.
    struct CommonDirs {
        answers: Vec<(PathBuf, PathBuf)>,
        asked: Vec<PathBuf>,
    }

    impl CommonDirs {
        fn of(answers: &[(&str, &str)]) -> CommonDirs {
            CommonDirs {
                answers: answers
                    .iter()
                    .map(|(path, common)| (PathBuf::from(path), PathBuf::from(common)))
                    .collect(),
                asked: Vec::new(),
            }
        }

        fn ask(&mut self, path: &Path) -> Option<PathBuf> {
            self.asked.push(path.to_path_buf());
            self.answers
                .iter()
                .find(|(answered, _)| answered == path)
                .map(|(_, common)| common.clone())
        }
    }

    /// `at_home` for the rules that are about paths alone: git knows no common directory.
    fn held(cwd: &str, top: Option<&str>, claims: &[Claim]) -> bool {
        at_home(Path::new(cwd), top.map(Path::new), claims, |_| None)
    }

    /// `at_home` inside the repository at `top`, with git answering from `dirs`.
    fn held_by(cwd: &str, top: &str, claims: &[Claim], dirs: &mut CommonDirs) -> bool {
        at_home(Path::new(cwd), Some(Path::new(top)), claims, |path| {
            dirs.ask(path)
        })
    }

    const APP: &str = "/repo/audrey-app";

    #[test]
    fn a_directory_under_a_registered_path_is_already_at_home() {
        let claims = [git(APP), folder("/notes")];
        assert!(held(APP, Some(APP), &claims));
        assert!(held("/repo/audrey-app/crates/api", Some(APP), &claims));
        assert!(held("/notes", None, &claims));
    }

    /// MUX-37. The start-up seed registered the home directory as a folder project, and every
    /// repository under it was then at home, so attach typed in one of them never asked. A
    /// folder project holds folders; a repository below it is a project of its own.
    #[test]
    fn a_repository_under_a_folder_project_is_not_at_home() {
        let claims = [folder("/home/u")];
        let top = Some("/home/u/domux");
        assert!(!held("/home/u/domux", top, &claims));
        assert!(!held("/home/u/domux/crates", top, &claims));
    }

    #[test]
    fn a_plain_folder_under_a_folder_project_is_still_at_home() {
        let claims = [folder("/home/u")];
        assert!(held("/home/u", None, &claims));
        assert!(held("/home/u/notes", None, &claims));
    }

    /// A git project holds everything under its root, so a subdirectory, a slot and a linked
    /// worktree made by hand beside the slots are all quiet, although each of the last two is
    /// a worktree with a top level of its own.
    #[test]
    fn a_directory_under_a_git_project_or_its_slot_is_still_at_home() {
        let workspace = "/repo/audrey-app/.domux/worktrees/workspace-1";
        let claims = [git(APP), slot(workspace)];
        assert!(held("/repo/audrey-app/crates", Some(APP), &claims));
        assert!(held(workspace, Some(workspace), &claims));
        let by_hand = "/repo/audrey-app/.worktrees/mux-37";
        assert!(held(by_hand, Some(by_hand), &claims));
    }

    /// A linked worktree made outside the project's root shares the project's common
    /// directory, which is what makes it the same repository. No registered path holds it,
    /// and before this the offer asked about it on every attach.
    #[test]
    fn a_linked_worktree_of_a_git_project_is_at_home_wherever_it_was_made() {
        let common = "/repo/audrey-app/.git";
        let feature = "/repo/app-feature";
        let mut dirs = CommonDirs::of(&[(APP, common), (feature, common)]);
        assert!(held_by(feature, feature, &[git(APP)], &mut dirs));
        assert!(held_by(
            "/repo/app-feature/crates",
            feature,
            &[git(APP)],
            &mut dirs
        ));

        // Under a folder project as well: the home project does not hold a repository, and
        // the git project beside the worktree does.
        let common = "/home/u/app/.git";
        let feature = "/home/u/app-feature";
        let mut dirs = CommonDirs::of(&[("/home/u/app", common), (feature, common)]);
        let claims = [folder("/home/u"), git("/home/u/app")];
        assert!(held_by(feature, feature, &claims, &mut dirs));
    }

    /// A second clone of the same remote is a repository of its own, with a common directory
    /// of its own, and is offered like any other.
    #[test]
    fn a_repository_with_a_common_directory_of_its_own_is_not_at_home() {
        let mut dirs = CommonDirs::of(&[
            ("/home/u/app", "/home/u/app/.git"),
            ("/home/u/app-2", "/home/u/app-2/.git"),
        ]);
        let claims = [folder("/home/u"), git("/home/u/app")];
        assert!(!held_by(
            "/home/u/app-2",
            "/home/u/app-2",
            &claims,
            &mut dirs
        ));
    }

    /// Only a git project's root is asked for its common directory. A folder record at a
    /// repository's root is not the repository's project (decision record 0010), and a slot
    /// shares its project's.
    #[test]
    fn only_a_git_projects_root_is_asked_for_its_common_directory() {
        let common = "/repo/audrey-app/.git";
        let feature = "/repo/app-feature";
        let workspace = "/repo/audrey-app-1";
        let mut dirs = CommonDirs::of(&[(APP, common), (feature, common), (workspace, common)]);
        let claims = [folder(APP), slot(workspace)];
        assert!(!held_by(feature, feature, &claims, &mut dirs));
        assert_eq!(dirs.asked, [PathBuf::from(feature)]);
    }

    /// Every common directory is a git process, and waiting on one takes about 10 ms. Most
    /// attaches are typed inside a registered path, so git is asked nothing when a path holds
    /// the directory or when it is in no repository, and it stops at the first root that
    /// holds it.
    #[test]
    fn git_is_asked_for_common_directories_only_when_no_path_holds_the_directory() {
        let common = "/repo/audrey-app/.git";
        let feature = "/repo/app-feature";
        let answers = [
            (APP, common),
            (feature, common),
            ("/repo/other", "/repo/other/.git"),
        ];
        let claims = [git("/repo/other"), git(APP), git("/repo/third")];

        let mut dirs = CommonDirs::of(&answers);
        assert!(held_by("/repo/audrey-app/crates", APP, &claims, &mut dirs));
        assert!(dirs.asked.is_empty(), "a path holds it: {:?}", dirs.asked);

        let mut dirs = CommonDirs::of(&answers);
        assert!(!at_home(Path::new("/notes"), None, &claims, |path| dirs.ask(path)));
        assert!(dirs.asked.is_empty(), "no repository: {:?}", dirs.asked);

        let mut dirs = CommonDirs::of(&answers);
        assert!(held_by(feature, feature, &claims, &mut dirs));
        assert_eq!(
            dirs.asked,
            [feature, "/repo/other", APP].map(PathBuf::from),
            "and not the root after the one that holds it"
        );

        let mut dirs = CommonDirs::of(&answers);
        assert!(!held_by(
            "/repo/unknown",
            "/repo/unknown",
            &claims,
            &mut dirs
        ));
        assert_eq!(
            dirs.asked,
            [PathBuf::from("/repo/unknown")],
            "no root, when git will not say the repository's own"
        );
    }

    /// The common directory only ever adds to what is at home. A submodule and a clone under a
    /// git project each have a common directory of their own, and they stay quiet as they
    /// were, because the project's root holds them by their paths (decision record 0041).
    #[test]
    fn a_submodule_or_a_clone_under_a_git_project_is_still_at_home() {
        let submodule = "/repo/audrey-app/vendor/lib";
        let clone = "/repo/audrey-app/fixtures/other";
        let mut dirs = CommonDirs::of(&[
            (APP, "/repo/audrey-app/.git"),
            (submodule, "/repo/audrey-app/.git/modules/lib"),
            (clone, "/repo/audrey-app/fixtures/other/.git"),
        ]);
        assert!(held_by(submodule, submodule, &[git(APP)], &mut dirs));
        assert!(held_by(clone, clone, &[git(APP)], &mut dirs));
    }

    /// A submodule checked out in a linked worktree keeps its git directory inside the
    /// checkout's, under `.git/worktrees/<name>/modules`. No path holds it and its common
    /// directory is not the project's, so it was asked about on every attach, although it is
    /// as much the project's as a submodule under the root. A git directory inside another
    /// repository's is only ever a worktree's or a submodule's.
    #[test]
    fn a_submodule_in_a_linked_worktree_of_a_git_project_is_at_home() {
        let lib = "/repo/app-f2/vendor/lib";
        let mut dirs = CommonDirs::of(&[
            (APP, "/repo/audrey-app/.git"),
            (
                lib,
                "/repo/audrey-app/.git/worktrees/app-f2/modules/vendor/lib",
            ),
        ]);
        assert!(held_by(lib, lib, &[git(APP)], &mut dirs));
        assert!(held_by(
            "/repo/app-f2/vendor/lib/src",
            lib,
            &[git(APP)],
            &mut dirs
        ));
    }

    /// A folder record at a repository's top level is what a state file written before decision
    /// record 0010 holds, and a folder record inside a repository is a folder registered before
    /// the repository around it was made. Neither has a repository below it, so both hold where
    /// the reader is.
    #[test]
    fn a_folder_project_at_or_inside_the_repository_is_at_home() {
        assert!(held("/repo/audrey-app/crates", Some(APP), &[folder(APP)]));
        assert!(held(
            "/repo/audrey-app/docs/notes",
            Some(APP),
            &[folder("/repo/audrey-app/docs")]
        ));
    }

    /// Component by component, not character by character: `/repo/audrey-app-2` is a
    /// different directory from `/repo/audrey-app` and must still be offered.
    #[test]
    fn a_directory_whose_name_merely_starts_the_same_is_not_at_home() {
        let claims = [git(APP)];
        assert!(!held("/repo/audrey-app-2", None, &claims));
        assert!(!held("/repo", None, &claims));
        assert!(!held("/elsewhere", None, &claims));
    }

    #[test]
    fn nothing_registered_means_every_directory_is_offered() {
        assert!(!held(APP, Some(APP), &[]));
        assert!(!held("/notes", None, &[]));
    }

    /// A workspace is held on its project's terms, so a slot of a git project holds the
    /// repositories under it and the `main` of a folder project does not. Only a project's
    /// path is its root.
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
        let elsewhere = "/elsewhere/audrey-app-1";
        let workspaces = [
            workspace("pr_0001", elsewhere),
            workspace("pr_0002", "/home/u"),
        ];
        let said: Vec<(PathBuf, bool, bool)> = claims(&projects, &workspaces)
            .into_iter()
            .map(|c| (c.path, c.git, c.root))
            .collect();
        assert_eq!(
            said,
            [
                (PathBuf::from(APP), true, true),
                (PathBuf::from("/home/u"), false, true),
                (PathBuf::from(elsewhere), true, false),
                (PathBuf::from("/home/u"), false, false),
            ]
        );
    }

    /// `--path-format` arrived in git 2.31, and an older git does not refuse a flag it does
    /// not know: it prints it back on a line of its own before its answer. So one full path is
    /// taken as it stands, anything else is asked again without the flag and read from the
    /// directory git was asked in, and a refusal is not asked again.
    #[test]
    fn the_common_directory_is_read_from_whichever_answer_git_gave() {
        let dir = Path::new("/repo/audrey-app/crates");
        let path = |p: &str| Some(PathBuf::from(p));
        let never = || -> Option<PathBuf> { panic!("git was asked again") };
        assert_eq!(
            common_dir_from(dir, path("/repo/audrey-app/.git"), never),
            path("/repo/audrey-app/.git")
        );
        assert_eq!(common_dir_from(dir, None, never), None);

        let echoed = path("--path-format=absolute\n../.git");
        assert_eq!(
            common_dir_from(dir, echoed.clone(), || path("../.git")),
            path("/repo/audrey-app/crates/../.git")
        );
        // A linked worktree's answer is a full path even without the flag.
        assert_eq!(
            common_dir_from(dir, echoed.clone(), || path("/repo/audrey-app/.git")),
            path("/repo/audrey-app/.git")
        );
        assert_eq!(common_dir_from(dir, echoed, || None), None);
        assert_eq!(common_dir_from(dir, path(".git"), || None), None);
    }

    /// Against real repositories: the checkout, a worktree made beside it and a subdirectory
    /// of either answer one common directory, a repository of its own answers another, and a
    /// plain folder answers none.
    #[test]
    fn every_worktree_of_one_repository_answers_the_same_common_directory() {
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
        let top = top_level_of(&inside).expect("the worktree is a repository");
        assert_eq!(top, feature.canonicalize().unwrap());
        assert_eq!(common_dir_of(&top).as_ref(), Some(&common));
        assert_eq!(common_dir_of(&inside).as_ref(), Some(&common));
        assert_ne!(common_dir_of(&other), Some(common));
        assert_eq!(common_dir_of(&plain), None);
        assert!(top_level_of(&plain).is_none());
    }

    /// Against real repositories: a submodule checked out in a linked worktree answers a common
    /// directory inside the checkout's, and the checkout's project holds it.
    #[test]
    fn a_submodule_in_a_linked_worktree_answers_a_common_directory_inside_the_checkouts() {
        use domux_server::testing::{git, repo_with_origin, repo_with_origin_at};
        let (tmp, app) = repo_with_origin("main");
        let lib = tmp.path().join("lib").join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        repo_with_origin_at(&lib, "main");
        // A submodule from a local path needs the file protocol, and its clone runs no hooks
        // from the machine the test runs on.
        let no_hooks = format!("core.hooksPath={}", tmp.path().join("no-hooks").display());
        let submodule = |dir: &Path, args: &[&str]| {
            let mut all = vec!["-c", "protocol.file.allow=always", "-c", &no_hooks];
            all.extend(["submodule", "-q"]);
            all.extend(args);
            git(dir, &all);
        };
        submodule(&app, &["add", lib.to_str().unwrap(), "vendor/lib"]);
        git(&app, &["commit", "-q", "-m", "Add lib"]);
        let f2 = tmp.path().join("app-f2");
        git(
            &app,
            &["worktree", "add", "-q", "-b", "f2", f2.to_str().unwrap()],
        );
        submodule(&f2, &["update", "--init"]);

        let checkout = common_dir_of(&app).expect("the checkout has a common directory");
        let inside = f2.join("vendor").join("lib").canonicalize().unwrap();
        let top = top_level_of(&inside).expect("the submodule is a repository");
        let its_own = common_dir_of(&top).expect("the submodule has a common directory");
        assert!(
            its_own.starts_with(&checkout) && its_own != checkout,
            "{} is not inside {}",
            its_own.display(),
            checkout.display()
        );
        let project = Claim {
            path: app.canonicalize().unwrap(),
            git: true,
            root: true,
        };
        assert!(at_home(&inside, Some(&top), &[project], common_dir_of));
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

        let top = top_level_of(&inside).expect("the directory is a repository");
        assert_eq!(top, repo.canonicalize().unwrap());
        assert_eq!(
            common_dir_of(&top),
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
        let terminal = std::io::Error::other("Input/output error");
        assert_eq!(
            Stopped::Asking(dir(), terminal.into()).to_string(),
            "Could not ask whether to register /home/u/my app: Input/output error. Run domux open '/home/u/my app' to register it."
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

    /// When the server would not give its records, the cause names what to do about the
    /// server, and that is the one action on the line: `domux open` goes through the same
    /// server, so naming it too would send the reader to a second command that fails the same
    /// way.
    #[test]
    fn an_offer_the_server_did_not_answer_names_only_the_servers_next_action() {
        let dir = || PathBuf::from("/home/u/my app");
        let unanswered = anyhow::anyhow!("early eof")
            .context("The server did not answer project.list on /s. Run domux server status.");
        assert_eq!(
            Stopped::Reading(dir(), unanswered).to_string(),
            "Could not ask whether to register /home/u/my app: early eof. The server did not answer project.list on /s. Run domux server status."
        );
        let stopped =
            anyhow::anyhow!("The server is not running. Start it with domux server start.");
        assert_eq!(
            Stopped::Reading(dir(), stopped).to_string(),
            "Could not ask whether to register /home/u/my app: The server is not running. Start it with domux server start."
        );
    }

    /// `{:#}` joins a context to its cause with a colon, which after a context that is already
    /// a sentence reads `status.: early eof`. Such a context goes after its causes instead; one
    /// that is not a sentence keeps the colon.
    #[test]
    fn a_cause_goes_before_a_context_that_is_already_a_sentence() {
        let under_a_sentence = anyhow::anyhow!("early eof")
            .context("The server did not answer x on /s. Run domux server status.");
        assert_eq!(
            one_line(&under_a_sentence),
            "early eof. The server did not answer x on /s. Run domux server status"
        );
        let under_a_phrase = anyhow::anyhow!("Permission denied (os error 13)").context("read /s");
        assert_eq!(
            one_line(&under_a_phrase),
            "read /s: Permission denied (os error 13)"
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
