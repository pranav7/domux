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
use domux_core::ids::WorkspaceId;
use domux_core::model::Model;
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

/// `project add` registers a path and prints the record it made on stdout. It is not `open`:
/// it does not switch, which is the whole of the difference between the two subcommands.
#[tokio::test]
async fn project_add_prints_the_record_it_made_and_leaves_the_client_where_it_was() {
    let h = Harness::start(Config::default(), 80, 24).await;
    let dir = tempfile::tempdir().unwrap();
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let client = h.client.clone();
    let before = where_the_client_is(&h.model(), &client);

    let added = run(domux2(&h).args(["project", "add", dir.path().to_str().unwrap()]))
        .await
        .ok();
    assert_eq!(added.err, "", "data goes to stdout (principle 12)");
    let record = added.json();
    assert_eq!(record["name"], name.as_str(), "{:?}", added.out);
    assert_eq!(
        record["kind"], "folder",
        "a folder with no repository in it"
    );

    let m = model_when(&h, "the project is registered", |m| {
        m.projects.iter().any(|p| p.name == name)
    })
    .await;
    assert_eq!(
        where_the_client_is(&m, &client),
        before,
        "add registers; open is the one that switches"
    );
}

/// The adoption line is said as soon as it is true, which is before the switch: a switch that
/// cannot happen does not unsay what the add already did.
///
/// The client is detached first, so `workspace.focus` has no view to act for and refuses.
/// That is also what `open` meets when it is typed in a terminal with nothing attached, which
/// is the case where a reader who was told nothing would never learn that three workspaces
/// had appeared.
#[tokio::test]
async fn open_names_what_it_adopted_even_when_there_is_nobody_to_switch() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_tmp, root) = repo_with_origin("main");
    for n in [1, 2] {
        std::fs::create_dir_all(slot_of(&root, n)).expect("make a worktree directory");
    }
    let client = h.client.clone();
    h.detach(client).await;
    model_when(&h, "the client goes", |m| m.most_recent_client().is_none()).await;

    let opened = run(domux2(&h).args(["open", root.to_str().unwrap()])).await;
    assert_eq!(opened.code, Some(1), "{}", opened.err);
    assert!(
        opened
            .err
            .starts_with("Adopted workspace-1, workspace-2.\n"),
        "the adoption is said first, because it is what already happened: {}",
        opened.err
    );
    assert!(
        opened.err.contains("no client is attached"),
        "and then the switch that could not happen: {}",
        opened.err
    );
    let m = model_when(&h, "the project is registered", |m| {
        m.projects.iter().any(|p| p.name == "audrey-app")
    })
    .await;
    assert_eq!(
        m.projects
            .iter()
            .find(|p| p.name == "audrey-app")
            .unwrap()
            .workspaces
            .len(),
        3,
        "main and the two slots it adopted"
    );
}

// ---------------------------------------------------------------- name

/// The names one workspace has, read back through `workspace list` rather than polled out of
/// the published model.
///
/// `workspace.rename` is answered by its handler on the core task, and the core takes its
/// messages in order, so a `workspace list` sent after the CLI has returned cannot be looking
/// at the state from before it. Polling the snapshot would work too, and it would make a wrong
/// implementation fail by running out of patience rather than by disagreeing: the timeout that
/// is there to catch a wedged core would be carrying the assertion, and a bound that fires is
/// a bound a loaded machine can fire on its own.
async fn name_of(h: &Harness, workspace: &WorkspaceId) -> Option<String> {
    let listed = run(domux2(h).args(["workspace", "list"])).await.ok();
    listed
        .json()
        .as_array()
        .expect("an array")
        .iter()
        .find(|w| w["id"] == workspace.as_str())
        .unwrap_or_else(|| panic!("no row for {workspace}"))["name"]
        .as_str()
        .map(str::to_string)
}

/// `workspace name` acts on the workspace `DOMUX_WORKSPACE` holds, and an empty name clears
/// it. `clear-name` is the same operation and reads the same variable.
///
/// The workspace it names is not the one the client is in: `git_project_with_two_slots`
/// registers its repository and makes two slots without moving anybody, so the client is
/// still in the harness's own project. A subcommand that ignored the environment and let the
/// server fall back to the view would name that one instead, and every assertion here would
/// fail. `elsewhere` is checked each time as well, because "it named the right one" and "it
/// named nothing" are not the same answer.
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
    assert_eq!(name_of(&h, &w1).await.as_deref(), Some("auth cleanup"));
    assert_eq!(name_of(&h, &elsewhere).await, None, "and only that one");

    run(domux2(&h)
        .env("DOMUX_WORKSPACE", w1.as_str())
        .args(["workspace", "name", ""]))
    .await
    .ok();
    assert_eq!(name_of(&h, &w1).await, None, "an empty name clears it");

    // `clear-name` is checked here, where the client is somewhere else, rather than in the
    // test that pairs it with `leader n`: there the client sits on this very workspace, so a
    // subcommand that sent no target at all would reach it anyway.
    run(domux2(&h)
        .env("DOMUX_WORKSPACE", w1.as_str())
        .args(["workspace", "name", "auth cleanup"]))
    .await
    .ok();
    assert_eq!(name_of(&h, &w1).await.as_deref(), Some("auth cleanup"));
    run(domux2(&h)
        .env("DOMUX_WORKSPACE", w1.as_str())
        .args(["workspace", "clear-name"]))
    .await
    .ok();
    assert_eq!(name_of(&h, &w1).await, None);
    assert_eq!(
        name_of(&h, &elsewhere).await,
        None,
        "and it did not reach for the view's workspace"
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
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    let scratch = slot_of(&root, 1).join("scratch.txt");
    std::fs::write(&scratch, "work").unwrap();

    // Named, with no variable set, so a target that came from the environment would be the
    // view's own workspace instead.
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

    // And with no target named, the variable is what says which workspace. The client is in
    // the harness's own project, whose `main` a clear refuses by name, so a subcommand that
    // sent no target would be refused in different words.
    let from_the_variable = run(domux2(&h)
        .env("DOMUX_WORKSPACE", w1.as_str())
        .args(["workspace", "clear"]))
    .await;
    assert_eq!(from_the_variable.code, Some(1));
    assert!(
        from_the_variable
            .err
            .contains("workspace-1 has uncommitted or unpushed changes"),
        "{}",
        from_the_variable.err
    );

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
    let made = made.json();
    assert_eq!(made["handle"], "workspace-3");
    assert_eq!(
        made["base"], "origin/main",
        "with no --base the default is origin/HEAD"
    );

    // `--base` is the one thing about a create that the answer reports and nothing else can:
    // the ref a slot was branched from is on no later row. A `main` that reached the server
    // answers `main` where the default answers `origin/main`, so the two are told apart by
    // the answer rather than by the flag having been typed.
    let based = run(domux2(&h).args([
        "workspace",
        "create",
        "--project",
        "audrey-app",
        "--base",
        "main",
    ]))
    .await
    .ok();
    let based = based.json();
    assert_eq!(based["handle"], "workspace-4");
    assert_eq!(based["base"], "main");

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
        [
            "main",
            "workspace-1",
            "workspace-2",
            "workspace-3",
            "workspace-4"
        ]
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

/// Every M2 subcommand and every flag it takes says what it does.
///
/// The first line of a clap help is the command's own description, so a command with none
/// starts straight at `Usage:`. A flag with none draws as its name and nothing else, which
/// is how `--yes` first shipped here: it read as a placeholder to anyone who had not already
/// read the code. Both are checked by shape rather than by matching the words, because a
/// check that matched the words would have to be rewritten every time the words improve.
///
/// `import` is not here: Task 23 created it and `cli_import` checks its own.
#[tokio::test]
async fn every_m2_subcommand_and_flag_has_help_that_says_what_it_does() {
    for args in [
        vec!["project", "--help"],
        vec!["project", "add", "--help"],
        vec!["project", "remove", "--help"],
        vec!["project", "list", "--help"],
        vec!["workspace", "--help"],
        vec!["workspace", "create", "--help"],
        vec!["workspace", "name", "--help"],
        vec!["workspace", "clear-name", "--help"],
        vec!["workspace", "clear", "--help"],
        vec!["workspace", "delete", "--help"],
        vec!["workspace", "list", "--help"],
        vec!["open", "--help"],
    ] {
        let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
        c.env("DOMUX_SOCKET", "/nonexistent/sock");
        let helped = run(c.args(&args)).await;
        assert_eq!(helped.code, Some(0), "{args:?}: {}", helped.err);
        let text = helped.out;
        let first = text.lines().next().unwrap_or_default();
        assert!(
            !first.is_empty() && !first.starts_with("Usage:"),
            "{args:?} has no description of its own: {text}"
        );
        let bare: Vec<&str> = text
            .lines()
            .filter(|line| {
                let words: Vec<&str> = line.split_whitespace().collect();
                // A flag's own line starts with its short or long form. A description line
                // that happens to end in `--force` starts with a word, so it is left alone.
                words.first().is_some_and(|w| w.starts_with('-'))
                    && words.last().is_some_and(|w| w.starts_with("--"))
            })
            .collect();
        assert!(
            bare.is_empty(),
            "{args:?} names a flag and says nothing about it: {bare:?}"
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
