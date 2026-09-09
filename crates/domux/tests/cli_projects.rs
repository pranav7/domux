//! End to end tests for `domux2 project`, `domux2 workspace` and `domux2 open`, run as the
//! process a person types against a harness server on a temp socket.
//!
//! The commands run through `tokio::process` rather than `std::process`, for the reason
//! `cli.rs` gives: the harness server lives on this test's own runtime, so a blocking
//! `output()` would stop the server it is waiting for.
//!
//! Every repository here is built by `tempfile::tempdir` through `repo_with_origin`, and
//! every path these tests destroy is inside one.

use domux_core::config::Config;
use domux_core::model::{Model, WorkspaceHandle};
use domux_server::testing::{repo_with_origin, Harness};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// The CLI pointed at this harness's socket, and at no pane: every test that wants a
/// location sets it itself.
fn domux2(h: &Harness) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
    c.env("DOMUX_SOCKET", h.socket_path());
    c.env_remove("DOMUX_TAB");
    c.env_remove("DOMUX_PANE");
    c.env_remove("DOMUX_WORKSPACE");
    c.env_remove("TMUX");
    c
}

/// What one run of the CLI came to, as the three things principle 12 separates.
struct Run {
    code: Option<i32>,
    out: String,
    err: String,
}

async fn run(cmd: &mut Command) -> Run {
    let out = cmd.output().await.expect("run domux2");
    Run {
        code: out.status.code(),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

impl Run {
    fn ok(self) -> Run {
        assert_eq!(self.code, Some(0), "{}", self.err);
        self
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("{e}: {:?}", self.out))
    }
}

/// An anti-hang bound, not a patience bound.
///
/// The CLI subcommands here answer in well under a second on an idle machine; the whole
/// binary is measured in this task's report. Two minutes is far outside the spread this
/// milestone has measured for a contention-sensitive test binary (0.55s, 52s and 60s on
/// consecutive runs of identical code), so a loaded machine cannot turn a survivor into a
/// kill by tripping this. It exists so a wedged core stops the test rather than the job.
const NO_HANG: Duration = Duration::from_secs(120);

/// The published model once `pred` holds.
///
/// The core answers a caller from inside the batch that changed the model and publishes the
/// snapshot at the end of that batch, so a read taken straight after the answer can be one
/// batch behind.
async fn model_when(h: &Harness, what: &str, pred: impl Fn(&Model) -> bool) -> Model {
    let deadline = tokio::time::Instant::now() + NO_HANG;
    loop {
        let m = h.model();
        if pred(&m) {
            return m;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The project the client is looking at, and the handle of the workspace inside it.
fn where_the_client_is(m: &Model, client: &domux_core::ids::ClientId) -> (String, String) {
    let workspace = m
        .client(client)
        .expect("the client is attached")
        .workspace
        .clone();
    let project = m
        .project_of_workspace(&workspace)
        .expect("the workspace is in a project");
    let handle = m
        .workspace(&workspace)
        .expect("the workspace is there")
        .handle
        .to_string();
    (project.name.clone(), handle)
}

fn slot_of(root: &Path, n: u32) -> PathBuf {
    root.join(format!(".domux/worktrees/workspace-{n}"))
}

// ---------------------------------------------------------------- open

/// `open` is the domain model's Open row: the path becomes a project and the client switches
/// to it. `project list` then prints what is registered as JSON on stdout (principle 12).
///
/// A plain folder, so nothing is adopted and `open` says nothing at all.
#[tokio::test]
async fn open_registers_a_path_and_switches_to_it_and_project_list_prints_json() {
    let h = Harness::start(Config::default(), 80, 24).await;
    let dir = tempfile::tempdir().unwrap();
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let opened = run(domux2(&h).args(["open", dir.path().to_str().unwrap()]))
        .await
        .ok();
    assert_eq!(opened.out, "", "open changed something and says nothing");
    assert_eq!(
        opened.err, "",
        "and it adopted nothing, so it names nothing"
    );

    let client = h.client.clone();
    let m = model_when(&h, "the client moves to the project it opened", |m| {
        where_the_client_is(m, &client).0 == name
    })
    .await;
    assert_eq!(
        where_the_client_is(&m, &client),
        (name.clone(), "main".to_string())
    );

    let listed = run(domux2(&h).args(["project", "list"])).await.ok();
    assert_eq!(
        listed.err, "",
        "data goes to stdout, messages to stderr (principle 12)"
    );
    let projects = listed.json();
    assert!(
        projects
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == name.as_str()),
        "data goes to stdout as JSON (principle 12): {:?}",
        listed.out
    );
}

/// The workspace `open` switches to is the one `project.add` answered, which is the project's
/// `main`.
///
/// The repository has two worktrees beside it, so the project it registers holds three
/// workspaces rather than one: with only `main` there, "the workspace the add answered", "the
/// first workspace" and "the last workspace" are all the same workspace and no assertion here
/// could tell them apart.
///
/// The two slots are also what `adopted` reports, and a reader who typed one path and got
/// three workspaces is told which ones arrived.
#[tokio::test]
async fn open_switches_to_main_and_names_the_slots_it_adopted() {
    let h = Harness::start(Config::default(), 80, 24).await;
    let (_tmp, root) = repo_with_origin("main");
    for n in [1, 2] {
        std::fs::create_dir_all(slot_of(&root, n)).expect("make a worktree directory");
    }

    let opened = run(domux2(&h).args(["open", root.to_str().unwrap()]))
        .await
        .ok();
    assert_eq!(opened.err, "Adopted workspace-1, workspace-2.\n");
    assert_eq!(
        opened.out, "",
        "which is a message, not data (principle 12)"
    );

    let client = h.client.clone();
    let m = model_when(&h, "the client moves to the project it opened", |m| {
        where_the_client_is(m, &client).0 == "audrey-app"
    })
    .await;
    assert_eq!(
        where_the_client_is(&m, &client),
        ("audrey-app".to_string(), "main".to_string()),
        "the add answered with main, and that is where the switch lands"
    );
}

// ---------------------------------------------------------------- name

/// `workspace name` acts on the workspace `DOMUX_WORKSPACE` holds, and an empty name clears
/// it.
///
/// The workspace it names is not the one the client is in: `git_project_with_two_slots`
/// registers its repository and makes two slots without moving anybody, so the client is
/// still in the harness's own project. A subcommand that ignored the environment and let the
/// server fall back to the view would name that one instead, and every assertion here would
/// fail.
#[tokio::test]
async fn workspace_name_reads_its_workspace_from_the_environment_and_an_empty_name_clears_it() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    let elsewhere = h.model().client(&client).unwrap().workspace.clone();
    assert_ne!(elsewhere, w1, "the fixture puts the client somewhere else");

    let named = run(domux2(&h).env("DOMUX_WORKSPACE", w1.as_str()).args([
        "workspace",
        "name",
        "auth cleanup",
    ]))
    .await
    .ok();
    assert_eq!(named.out, "", "quiet on success");
    assert_eq!(named.err, "");
    let m = model_when(&h, "the name arrives", |m| {
        m.workspace(&w1).unwrap().name.is_some()
    })
    .await;
    assert_eq!(
        m.workspace(&w1).unwrap().name.as_deref(),
        Some("auth cleanup")
    );
    assert_eq!(
        m.workspace(&elsewhere).unwrap().name,
        None,
        "and only that workspace"
    );

    run(domux2(&h)
        .env("DOMUX_WORKSPACE", w1.as_str())
        .args(["workspace", "name", ""]))
    .await
    .ok();
    let m = model_when(&h, "the name goes", |m| {
        m.workspace(&w1).unwrap().name.is_none()
    })
    .await;
    assert_eq!(
        m.workspace(&w1).unwrap().name,
        None,
        "an empty name clears it"
    );
}

/// `leader n` and `workspace clear-name` are one operation, so they reach one handler.
///
/// Both act on the same workspace, and the client is moved onto it first so the key has
/// somewhere to press: `leader n` carries no target and takes the client's own workspace.
/// That the subcommand reads `DOMUX_WORKSPACE` rather than falling back the same way is the
/// test above; this one is only about the two paths coming to the same place.
#[tokio::test]
async fn the_clear_name_key_and_the_clear_name_subcommand_both_clear_the_name() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.focus",
        serde_json::json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();

    for clear_it in ["key", "subcommand"] {
        h.api(
            "workspace.rename",
            serde_json::json!({"workspace": w1.as_str(), "name": "auth cleanup"}),
        )
        .await
        .unwrap();
        model_when(&h, "the name arrives", |m| {
            m.workspace(&w1).unwrap().name.is_some()
        })
        .await;

        if clear_it == "key" {
            h.key(client.clone(), "C-a").await;
            h.key(client.clone(), "n").await;
        } else {
            run(domux2(&h)
                .env("DOMUX_WORKSPACE", w1.as_str())
                .args(["workspace", "clear-name"]))
            .await
            .ok();
        }

        let m = model_when(&h, &format!("the {clear_it} clears the name"), |m| {
            m.workspace(&w1).unwrap().name.is_none()
        })
        .await;
        assert_eq!(
            m.workspace(&w1).unwrap().name,
            None,
            "cleared by the {clear_it}"
        );
    }
}

// ---------------------------------------------------------------- delete

/// Interface spec 12.22: `domux2 workspace delete workspace-1` prints the confirmation and
/// fails unless `--yes` is given (principle 10: non-interactive use gets the whole
/// consequence, not a one-line refusal).
///
/// The question is the server's own. `deletion_copy` builds it once for both surfaces, and
/// the sentence asserted here is the one
/// `workspace_clear_delete::deleting_without_yes_asks_and_changes_nothing` asserts on the API,
/// less the server's `Answer with --yes` tail: this reads `data["confirmation"]`, which is that
/// sentence without it, and says how to answer in the words of the flag a shell reader has.
///
/// The harness registers no fact provider, so no branch fact has arrived and the question says
/// "its local branch" rather than naming the handle. The brief's sample asserted
/// `the local branch workspace-1`, which is what the question says only once a branch provider
/// has answered.
#[tokio::test]
async fn delete_without_yes_prints_the_confirmation_on_stderr_and_exits_one() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;

    let asked = run(domux2(&h).args(["workspace", "delete", "workspace-1"])).await;
    assert_eq!(asked.code, Some(1));
    assert_eq!(
        asked.out, "",
        "and nothing on stdout: there is no data (principle 12)"
    );
    let lines: Vec<&str> = asked.err.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some(
            "Delete workspace-1? Removes the worktree at .domux/worktrees/workspace-1 and its \
             local branch and closes 1 tab. The remote branch and any pull request stay."
        ),
        "{}",
        asked.err
    );
    assert!(
        asked.err.contains(
            "Removes:\n  the worktree at .domux/worktrees/workspace-1\n  its local branch"
        ),
        "{}",
        asked.err
    );
    assert!(
        asked
            .err
            .contains("Keeps:\n  the remote branch and any pull request"),
        "{}",
        asked.err
    );
    assert_eq!(
        lines.last().copied(),
        Some("Run it again with --yes."),
        "on the same stream as the question: {}",
        asked.err
    );
    assert!(
        !asked.err.contains("Answer with --yes"),
        "the flag the reader has, said once: {}",
        asked.err
    );
    // Asking is not doing, and an exit code alone does not prove that.
    assert!(slot_of(&root, 1).is_dir(), "the worktree is still there");
    assert!(h.model().workspace(&w1).is_some(), "and so is its record");

    run(domux2(&h).args(["workspace", "delete", "workspace-1", "--yes"]))
        .await
        .ok();
    model_when(&h, "the record goes", |m| m.workspace(&w1).is_none()).await;
    assert!(!slot_of(&root, 1).exists(), "and the worktree with it");
}

/// `--force` reaches the server. Without it a slot holding work is refused, with the job's own
/// words, and the work is still there afterwards.
#[tokio::test]
async fn a_workspace_holding_work_needs_force_and_keeps_its_work_until_it_gets_one() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, w2) = h.git_project_with_two_slots().await;
    let scratch = slot_of(&root, 2).join("scratch.txt");
    std::fs::write(&scratch, "work").unwrap();

    let refused = run(domux2(&h).args(["workspace", "delete", "workspace-2", "--yes"])).await;
    assert_eq!(refused.code, Some(1));
    assert!(
        refused.err.contains(
            "workspace-2 has uncommitted or unpushed changes; delete it with --force or commit \
             and push first"
        ),
        "{}",
        refused.err
    );
    assert!(scratch.is_file(), "and the work is still there");

    run(domux2(&h).args(["workspace", "delete", "workspace-2", "--yes", "--force"]))
        .await
        .ok();
    model_when(&h, "the record goes", |m| m.workspace(&w2).is_none()).await;
    assert!(!slot_of(&root, 2).exists());
}

// ---------------------------------------------------------------- clear

/// A clear does not ask a shell, and this is the test for that sentence rather than for the
/// code under it.
///
/// Whether there is anything to lose is `git::is_dirty`, which shells out, so only the job can
/// answer it: the refusal is the job's one line and it names `--yes` itself. There is no
/// `Removes:` list, because the server sent no question to lay out.
#[tokio::test]
async fn clear_does_not_ask_from_a_shell_and_yes_reaches_the_job() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    let scratch = slot_of(&root, 1).join("scratch.txt");
    std::fs::write(&scratch, "work").unwrap();

    let refused = run(domux2(&h).args(["workspace", "clear", "workspace-1"])).await;
    assert_eq!(refused.code, Some(1));
    assert!(
        refused.err.contains(
            "workspace-1 has uncommitted or unpushed changes; clear it with --yes to throw them \
             away"
        ),
        "{}",
        refused.err
    );
    assert!(
        !refused.err.contains("Removes:"),
        "a refusal that is not a question is one line: {}",
        refused.err
    );
    assert!(scratch.is_file(), "and nothing was thrown away");

    run(domux2(&h).args(["workspace", "clear", "workspace-1", "--yes"]))
        .await
        .ok();
    assert!(
        !scratch.exists(),
        "--yes reaches the job, which cleans the tree"
    );
}

// ---------------------------------------------------------------- project remove

/// `project remove` asks the same way, from the same printer: one question laid out, its two
/// lists under their labels, and the flag that answers it.
#[tokio::test]
async fn project_remove_without_yes_prints_the_confirmation_and_keeps_the_project() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;

    let asked = run(domux2(&h).args(["project", "remove", "audrey-app"])).await;
    assert_eq!(asked.code, Some(1));
    assert_eq!(asked.out, "");
    let lines: Vec<&str> = asked.err.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("Remove audrey-app? It has 3 workspaces. The folder and its worktrees stay on disk."),
        "{}",
        asked.err
    );
    assert!(
        asked
            .err
            .contains("Removes:\n  the record of audrey-app and its 3 workspaces"),
        "{}",
        asked.err
    );
    assert!(
        asked.err.contains("Keeps:\n  the folder at "),
        "{}",
        asked.err
    );
    assert_eq!(lines.last().copied(), Some("Run it again with --yes."));
    assert!(
        h.model().projects.iter().any(|p| p.name == "audrey-app"),
        "and asking removed nothing"
    );
}

// ---------------------------------------------------------------- create and list

/// `workspace create` and `workspace list` print their answers as JSON on stdout, and
/// `--project` scopes the list.
///
/// The unscoped list has to hold more than the scoped one, or a `--project` that was dropped
/// on the way to the server would be invisible: the harness always holds a second project of
/// its own, whose `main` is in the first answer and not the second.
#[tokio::test]
async fn workspace_create_and_list_print_json_and_project_scopes_the_list() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;

    let made = run(domux2(&h).args(["workspace", "create", "--project", "audrey-app"]))
        .await
        .ok();
    assert_eq!(made.json()["handle"], "workspace-3", "{:?}", made.out);

    let scoped = run(domux2(&h).args(["workspace", "list", "--project", "audrey-app"]))
        .await
        .ok();
    assert_eq!(scoped.err, "", "data goes to stdout (principle 12)");
    let rows = scoped.json();
    let handles: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["handle"].as_str().unwrap())
        .collect();
    assert_eq!(
        handles,
        ["main", "workspace-1", "workspace-2", "workspace-3"]
    );

    let all = run(domux2(&h).args(["workspace", "list"])).await.ok();
    assert_eq!(
        all.json().as_array().unwrap().len(),
        handles.len() + 1,
        "with no project it lists every project's, the harness's own included: {:?}",
        all.out
    );
}

// ---------------------------------------------------------------- help

/// Every M2 subcommand's help says what it does, in the words of this repository.
///
/// `import` is not here: Task 23 created it and `cli_import` checks its own.
#[tokio::test]
async fn every_m2_subcommand_has_help_that_names_what_it_does() {
    for args in [
        vec!["project", "--help"],
        vec!["project", "remove", "--help"],
        vec!["workspace", "--help"],
        vec!["workspace", "delete", "--help"],
        vec!["workspace", "clear", "--help"],
        vec!["open", "--help"],
    ] {
        let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
        c.env("DOMUX_SOCKET", "/nonexistent/sock");
        let helped = run(c.args(&args)).await;
        assert_eq!(helped.code, Some(0), "{args:?}: {}", helped.err);
        let text = helped.out;
        assert!(
            text.to_lowercase().contains(args[0]),
            "{args:?} does not name the command: {text}"
        );
        assert!(
            !text.contains('\u{2014}'),
            "{args:?} help still has an em dash: {text}"
        );
        assert!(
            !text.to_lowercase().contains("to do"),
            "{args:?} help still has a placeholder: {text}"
        );
    }
}

/// The handle grammar this file names in its assertions is the model's, not a string these
/// tests invented: `workspace-1` is a slot and `main` is the project's own checkout.
#[test]
fn the_handles_these_tests_name_are_the_model_s() {
    assert_eq!(WorkspaceHandle::Main.to_string(), "main");
    assert_eq!(WorkspaceHandle::Slot(1).to_string(), "workspace-1");
}
