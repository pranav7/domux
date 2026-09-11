//! End to end tests for `domux import v1`, run as the process a person types against a
//! harness server on a temp socket.
//!
//! Every test builds its own session files in a temp directory and passes `--from`, so
//! nothing here reads V1's real state directory. `domux` also runs with `HOME` pointed at a
//! temp directory, so a bug that ignored `--from` would read an empty directory rather than
//! the author's own sessions.

use domux_core::config::Config;
use domux_server::process::ForegroundProcess;
use domux_server::testing::{repo_with_origin, Harness};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::Command;

/// The CLI pointed at this harness's socket, at no pane, and at a home directory with
/// nothing in it.
fn domux(h: &Harness, home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_domux"));
    c.env("DOMUX_SOCKET", h.socket_path());
    c.env("HOME", home);
    c.env_remove("DOMUX_TAB");
    c.env_remove("DOMUX_PANE");
    c.env_remove("DOMUX_WORKSPACE");
    c.env_remove("TMUX");
    c
}

/// A git project with a `workspace-1` worktree directory beside it, and V1 session files
/// describing both, all under one temp directory.
struct Fixture {
    _repo_tmp: tempfile::TempDir,
    tmp: tempfile::TempDir,
    root: PathBuf,
    slot: PathBuf,
}

impl Fixture {
    fn build() -> Fixture {
        let (repo_tmp, root) = repo_with_origin("main");
        let slot = root.join(".domux/worktrees/workspace-1");
        std::fs::create_dir_all(slot.join("frontend")).expect("make the worktree directory");
        let tmp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(tmp.path().join("sessions")).expect("make the sessions directory");
        let fixture = Fixture {
            _repo_tmp: repo_tmp,
            tmp,
            root,
            slot,
        };
        fixture.write_main();
        fixture.write_slot("comments", &fixture.slot.join("frontend"));
        fixture
    }

    fn sessions(&self) -> PathBuf {
        self.tmp.path().join("sessions")
    }

    fn home(&self) -> PathBuf {
        self.tmp.path().join("home")
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.sessions().join(name), text).expect("write a session file");
    }

    /// V1's session for the main checkout: a name, and one window running an agent.
    fn write_main(&self) {
        let root = self.root.display();
        self.write(
            "audrey-app.json",
            &format!(
                r#"{{
  "name": "audrey-app",
  "root": "{root}",
  "todo_path": "/state/by-path/audrey-app.md",
  "label": "audit-harness",
  "workspace": "occupied",
  "windows": [
    {{ "index": 1, "name": "agent-harness", "cwd": "{root}", "agent": "claude" }}
  ],
  "created_at": "2026-05-19T19:05:15+01:00",
  "updated_at": "2026-09-04T22:17:27+01:00"
}}"#
            ),
        );
    }

    /// V1's session for the slot: two windows, the second of them in `second_cwd`.
    fn write_slot(&self, first: &str, second_cwd: &Path) {
        let slot = self.slot.display();
        let second = second_cwd.display();
        self.write(
            "workspace-1.json",
            &format!(
                r#"{{
  "name": "audrey-app-workspace-1",
  "root": "{slot}",
  "label": "reviewer",
  "workspace": "occupied",
  "windows": [
    {{ "index": 1, "name": "{first}", "cwd": "{slot}", "agent": "" }},
    {{ "index": 2, "name": "server", "cwd": "{second}", "agent": "" }}
  ],
  "created_at": "2026-08-01T09:12:00+01:00",
  "updated_at": "2026-09-04T18:40:11+01:00"
}}"#
            ),
        );
    }

    /// What the report says for this fixture after a real run.
    fn expected_report(&self) -> String {
        self.report_with("Imported")
    }

    /// What it says after a dry run. Only the summary's verb differs, so a reader can hold
    /// the two outputs side by side and see one word change.
    fn expected_dry_report(&self) -> String {
        self.report_with("Would import")
    }

    fn report_with(&self, verb: &str) -> String {
        format!(
            "{root}\n  main, named audit-harness, tabs: agent-harness\n  workspace-1, named \
             reviewer, tabs: comments, server\n{verb} 1 project, 2 workspaces, 3 tabs.\n\
             Skipped 1: audrey-app (its window agent-harness runs the agent claude, which V2 \
             has no home for until M3).\n",
            root = self.root.display()
        )
    }
}

/// `import v1 --from <dir>`, and what it printed.
async fn import(
    h: &Harness,
    f: &Fixture,
    extra: &[&str],
) -> (std::process::Output, String, String) {
    let mut command = domux(h, &f.home());
    command.args(["import", "v1", "--from"]);
    command.arg(f.sessions());
    command.args(extra);
    let out = command.output().await.expect("run the import");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (out, stdout, stderr)
}

/// The workspaces of the project the fixture registered, as `(handle, name)`.
async fn imported_workspaces(h: &mut Harness, f: &Fixture) -> Vec<(String, Option<String>)> {
    let root = f.root.canonicalize().expect("the repository is there");
    let projects = h
        .api("project.list", json!({}))
        .await
        .expect("project.list");
    let Some(project) = projects
        .as_array()
        .expect("an array")
        .iter()
        .find(|p| Path::new(p["root"].as_str().expect("a root")) == root)
    else {
        return Vec::new();
    };
    let rows = h
        .api(
            "workspace.list",
            json!({ "project": project["id"].as_str().expect("an id") }),
        )
        .await
        .expect("workspace.list");
    rows.as_array()
        .expect("an array")
        .iter()
        .map(|w| {
            (
                w["handle"].as_str().expect("a handle").to_string(),
                w["name"].as_str().map(str::to_string),
            )
        })
        .collect()
}

/// The id of one of the imported project's workspaces.
async fn workspace_id(h: &mut Harness, f: &Fixture, handle: &str) -> String {
    let root = f.root.canonicalize().expect("the repository is there");
    let projects = h
        .api("project.list", json!({}))
        .await
        .expect("project.list");
    let project = projects
        .as_array()
        .expect("an array")
        .iter()
        .find(|p| Path::new(p["root"].as_str().expect("a root")) == root)
        .expect("the project was registered");
    let rows = h
        .api(
            "workspace.list",
            json!({ "project": project["id"].as_str().expect("an id") }),
        )
        .await
        .expect("workspace.list");
    rows.as_array()
        .expect("an array")
        .iter()
        .find(|w| w["handle"] == handle)
        .expect("the workspace was registered")["id"]
        .as_str()
        .expect("an id")
        .to_string()
}

/// The tab names of one workspace, in order.
async fn tab_names(h: &mut Harness, f: &Fixture, handle: &str) -> Vec<Option<String>> {
    let root = f.root.canonicalize().expect("the repository is there");
    let projects = h
        .api("project.list", json!({}))
        .await
        .expect("project.list");
    let project = projects
        .as_array()
        .expect("an array")
        .iter()
        .find(|p| Path::new(p["root"].as_str().expect("a root")) == root)
        .expect("the project was registered");
    let rows = h
        .api(
            "workspace.list",
            json!({ "project": project["id"].as_str().expect("an id") }),
        )
        .await
        .expect("workspace.list");
    let workspace = rows
        .as_array()
        .expect("an array")
        .iter()
        .find(|w| w["handle"] == handle)
        .expect("the workspace was registered");
    let tabs = h
        .api(
            "tab.list",
            json!({ "workspace": workspace["id"].as_str().expect("an id") }),
        )
        .await
        .expect("tab.list");
    tabs.as_array()
        .expect("an array")
        .iter()
        .map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn import_v1_creates_the_projects_workspaces_names_and_tabs_v1_recorded() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    assert!(
        imported_workspaces(&mut h, &f).await.is_empty(),
        "nothing is registered before the import"
    );

    let (out, stdout, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(stdout, f.expected_report());
    assert_eq!(stderr, "", "a run with nothing wrong says nothing");

    assert_eq!(
        imported_workspaces(&mut h, &f).await,
        vec![
            ("main".to_string(), Some("audit-harness".to_string())),
            ("workspace-1".to_string(), Some("reviewer".to_string())),
        ],
        "the worktree already on disk is adopted and both labels become names"
    );
    // The tab `project.add` makes for every new workspace is there too, unnamed, because no
    // window of V1's matched it. The named ones follow it in V1's index order.
    assert_eq!(
        tab_names(&mut h, &f, "main").await,
        vec![None, Some("agent-harness".to_string())]
    );
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![
            None,
            Some("comments".to_string()),
            Some("server".to_string())
        ]
    );
}

#[tokio::test]
async fn a_dry_run_says_what_it_would_do_and_changes_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();

    let (out, dry_stdout, dry_stderr) = import(&h, &f, &["--dry-run"]).await;
    assert!(out.status.success(), "{dry_stderr}");
    assert_eq!(dry_stdout, f.expected_dry_report());
    assert!(
        imported_workspaces(&mut h, &f).await.is_empty(),
        "a dry run registers nothing"
    );

    // The summary carries its own tense, so stdout on its own is never a record of work
    // that did not happen. `import v1 --dry-run > plan.txt` keeps exactly this text.
    assert!(
        !dry_stdout.contains("Imported "),
        "a dry run imported nothing: {dry_stdout}"
    );
    assert!(
        dry_stdout.contains("Would import 1 project, 2 workspaces, 3 tabs."),
        "{dry_stdout}"
    );

    let (out, stdout, _) = import(&h, &f, &[]).await;
    assert!(out.status.success());
    assert_eq!(stdout, f.expected_report());
    assert_eq!(imported_workspaces(&mut h, &f).await.len(), 2);

    // The two outputs differ by one line, and that line differs by its verb alone. That is
    // what makes a dry run worth holding beside the run that follows it.
    let differing: Vec<(&str, &str)> = dry_stdout
        .lines()
        .zip(stdout.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        differing,
        vec![(
            "Would import 1 project, 2 workspaces, 3 tabs.",
            "Imported 1 project, 2 workspaces, 3 tabs."
        )]
    );
    assert_eq!(dry_stdout.lines().count(), stdout.lines().count());
}

#[tokio::test]
async fn a_second_run_adds_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();

    let (out, first, _) = import(&h, &f, &[]).await;
    assert!(out.status.success());
    let after_one = tab_names(&mut h, &f, "workspace-1").await;

    let (out, second, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(second, first);
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        after_one,
        "every planned tab was found rather than made again"
    );
    assert_eq!(
        tab_names(&mut h, &f, "main").await,
        vec![None, Some("agent-harness".to_string())]
    );
    assert_eq!(imported_workspaces(&mut h, &f).await.len(), 2);
}

/// The identity is the pair, so a planned tab that shares a name with a tab in another
/// directory is a different tab and arrives. Matching on the name alone would find the
/// `comments` tab from the first run and create nothing.
#[tokio::test]
async fn a_planned_tab_whose_name_is_taken_in_another_directory_is_still_created() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![
            None,
            Some("comments".to_string()),
            Some("server".to_string())
        ]
    );

    // One window, called `comments` as before and in the directory `server` was in. The
    // name is unchanged and the directory is not.
    f.write(
        "workspace-1.json",
        &format!(
            r#"{{
  "name": "audrey-app-workspace-1",
  "root": "{slot}",
  "label": "reviewer",
  "windows": [
    {{ "index": 1, "name": "comments", "cwd": "{moved}", "agent": "" }}
  ]
}}"#,
            slot = f.slot.display(),
            moved = f.slot.join("frontend").display()
        ),
    );

    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![
            None,
            Some("comments".to_string()),
            Some("server".to_string()),
            Some("comments".to_string())
        ],
        "the second comments tab is in a different directory, so it is a different tab"
    );
}

#[tokio::test]
async fn a_sessions_directory_that_is_not_there_fails() {
    let h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    let out = domux(&h, &f.home())
        .args(["import", "v1", "--from"])
        .arg(f.tmp.path().join("not-here"))
        .output()
        .await
        .expect("run the import");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "no report for a run that read nothing"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not-here"), "{stderr}");
}

#[tokio::test]
async fn a_session_file_that_will_not_read_is_named_and_the_others_still_arrive() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    f.write("broken.json", "{ this is not json");

    let (out, stdout, stderr) = import(&h, &f, &[]).await;
    assert_eq!(
        out.status.code(),
        Some(1),
        "a file that would not read failed"
    );
    assert!(stderr.contains("broken.json"), "{stderr}");
    assert!(
        stderr.contains("Not imported: 1 session file that would not read."),
        "the failure line counts files and says so: {stderr}"
    );
    assert!(
        !stderr.contains("project"),
        "no project was refused, so none is claimed: {stderr}"
    );
    assert_eq!(
        stdout,
        f.expected_report(),
        "the sessions that did read still came across"
    );
    assert_eq!(imported_workspaces(&mut h, &f).await.len(), 2);
}

/// Two of V1's windows can share a name and a directory, and they are two tabs. The
/// applier walks the workspace's tabs and consumes each match once rather than testing
/// membership, so the second planned tab cannot match the tab the first one matched.
///
/// **The shape of this test is the whole point.** One window is imported first, so the
/// workspace holds exactly one tab called `comments`. Then the session grows a second
/// window with the same name and directory, which is what happens when the author opens
/// another window in V1 between two imports. With matches consumed, the first planned tab
/// takes the tab that is there and the second finds nothing and is created. Without, both
/// planned tabs match the same one tab and the second window is silently lost.
///
/// Two existing and two planned is the count at which the two implementations agree, so a
/// fixture built that way passes either way. Measured: it did, and the mutant survived it.
///
/// It also has to go through the subcommand. The planner's own tests prove it emits two
/// identical `PlannedTab`s; the consuming happens in `apply_tabs`, and a test on the
/// planner cannot fail for a bug in the applier.
#[tokio::test]
async fn a_second_window_sharing_a_name_and_a_directory_is_a_second_tab() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    let slot = f.slot.display().to_string();
    let session = |windows: &str| {
        format!(
            r#"{{
  "name": "audrey-app-workspace-1",
  "root": "{slot}",
  "label": "reviewer",
  "windows": [{windows}]
}}"#
        )
    };
    let one = format!(r#"{{ "index": 1, "name": "comments", "cwd": "{slot}", "agent": "" }}"#);
    let two = format!(r#"{{ "index": 2, "name": "comments", "cwd": "{slot}", "agent": "" }}"#);

    f.write("workspace-1.json", &session(&one));
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![None, Some("comments".to_string())],
        "one window, one tab beside the one the workspace was born with"
    );

    f.write("workspace-1.json", &session(&format!("{one},{two}")));
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![
            None,
            Some("comments".to_string()),
            Some("comments".to_string())
        ],
        "the second window is a second tab; collapsing them would lose it"
    );

    // And a third run over the same two windows adds neither back.
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![
            None,
            Some("comments".to_string()),
            Some("comments".to_string())
        ]
    );
}

/// V1's directory holds more than session files, and anything that is not a `.json` is not
/// a session that failed to read. Without the extension filter this file would be read,
/// fail to parse, and leave the run reporting a session it never had.
#[tokio::test]
async fn a_file_that_is_not_a_session_file_is_not_read_at_all() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    std::fs::write(f.sessions().join("README"), "not a session\n").expect("write");
    std::fs::write(f.sessions().join("audrey-app.json.tmp"), "{ broken").expect("write");

    let (out, stdout, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(stderr, "", "neither file is a session that would not read");
    assert_eq!(stdout, f.expected_report());
    assert_eq!(imported_workspaces(&mut h, &f).await.len(), 2);
}

/// A project the server will not take is reported, and the rest of the import still runs.
///
/// The input is a session whose root is a relative path. The planner asks `Path::is_dir`,
/// which resolves against the process running the import; the server answers `project.add`
/// with `std::fs::canonicalize`, which resolves against the server's own directory. The two
/// need not agree, so a root that is there for one is missing for the other. That is the
/// disagreement the `refused` branch exists for, and it is the one way to reach it that does
/// not depend on file permissions, which differ between this machine and the Linux targets
/// CI runs.
#[tokio::test]
async fn a_project_the_server_will_not_take_is_reported_and_the_others_still_arrive() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    // Real for the import, which runs with its working directory here, and not resolvable
    // by the server, which runs somewhere else entirely.
    std::fs::create_dir_all(f.tmp.path().join("elsewhere")).expect("make the directory");
    f.write(
        "elsewhere.json",
        r#"{ "name": "elsewhere", "root": "elsewhere", "label": "relative",
             "windows": [{ "index": 1, "name": "one", "cwd": "elsewhere", "agent": "" }] }"#,
    );

    let mut command = domux(&h, &f.home());
    command.current_dir(f.tmp.path());
    command.args(["import", "v1", "--from"]);
    command.arg(f.sessions());
    let out = command.output().await.expect("run the import");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    assert_eq!(
        out.status.code(),
        Some(1),
        "a project was refused: {stderr}"
    );
    assert!(
        stderr.contains("Could not add elsewhere"),
        "the refusal names the project: {stderr}"
    );
    // One project and no unreadable file, so the line says "1 project". The old line added
    // the two counts and called the total sessions, which names a number of one thing
    // about another: one refused project can hold several of V1's sessions.
    assert!(
        stderr.contains("Not imported: 1 project."),
        "the failure line counts projects and says so: {stderr}"
    );
    assert!(
        !stderr.contains("session"),
        "no session file failed to read, so none is claimed: {stderr}"
    );
    assert_eq!(
        stdout,
        f.expected_report(),
        "the refused project is absent from the report and from its counts, and the \
         project that worked is still there"
    );
    assert_eq!(
        imported_workspaces(&mut h, &f).await.len(),
        2,
        "the import carried on past the refusal"
    );
}

/// A V1 session whose checkout is gone comes across as a skip the author can read, and the
/// run still succeeds.
///
/// This is the skip the author is most likely to ever see, because V1's session files
/// accumulate while repositories get deleted. The planner's own tests cover the rule, but
/// they hand `plan` the predicate themselves, so they are on the near side of the mutant:
/// the subcommand's `&|p| p.is_dir()` can be replaced with `&|_| true` and every one of
/// them stays green. This test is the far side of it.
#[tokio::test]
async fn a_session_whose_checkout_is_gone_is_skipped_with_its_reason() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    // Beside the repository rather than under a second temp directory, so the two roots
    // sort against each other predictably: `audrey-app` before `gone-repo`.
    let gone = f
        .root
        .parent()
        .expect("the repository has a parent")
        .join("gone-repo");
    assert!(
        !gone.exists(),
        "the point of the fixture is that it is not there"
    );
    f.write(
        "gone.json",
        &format!(
            r#"{{ "name": "gone", "root": "{}", "label": "old work",
                  "windows": [{{ "index": 1, "name": "one", "cwd": "{}", "agent": "" }}] }}"#,
            gone.display(),
            gone.display()
        ),
    );

    let (out, stdout, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "a skip is not a failure: {stderr}");
    assert_eq!(stderr, "", "nothing failed, so nothing is on stderr");
    assert_eq!(
        stdout,
        format!(
            "{root}\n  main, named audit-harness, tabs: agent-harness\n  workspace-1, named \
             reviewer, tabs: comments, server\nImported 1 project, 2 workspaces, 3 tabs.\n\
             Skipped 2: audrey-app (its window agent-harness runs the agent claude, which V2 \
             has no home for until M3), gone (root is gone).\n",
            root = f.root.display()
        ),
        "the session that is gone is named with its reason, and nothing of it is planned"
    );
    assert_eq!(
        imported_workspaces(&mut h, &f).await.len(),
        2,
        "no project was registered for the root that is not there"
    );
}

/// A tab reports the directory of its focused pane, not of whichever pane the layout holds
/// first. The two are the same for every tab this import makes, so the fixture splits one.
#[tokio::test]
async fn a_tabs_directory_is_its_focused_panes() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");

    let workspace = workspace_id(&mut h, &f, "workspace-1").await;
    let tabs = h
        .api("tab.list", json!({ "workspace": workspace }))
        .await
        .expect("tab.list");
    let comments = tabs
        .as_array()
        .expect("an array")
        .iter()
        .find(|t| t["name"] == "comments")
        .expect("the comments tab")
        .clone();
    let first_pane = comments["panes"][0].as_str().expect("a pane").to_string();
    // As written, not resolved: the pane was spawned at the directory V1's file names and
    // nothing has resolved it since.
    assert_eq!(
        Path::new(comments["cwd"].as_str().expect("a cwd")),
        f.slot,
        "one pane, so focused and first are the same pane"
    );

    // Split it into a directory the first pane is not in. `split_pane` focuses the new
    // pane, so the tab's focused pane and its first pane now differ, and only then can the
    // two answers be told apart.
    let elsewhere = f.slot.join("frontend");
    h.api(
        "pane.split",
        json!({ "pane": first_pane, "dir": "right", "cwd": elsewhere }),
    )
    .await
    .expect("pane.split");

    let tabs = h
        .api("tab.list", json!({ "workspace": workspace }))
        .await
        .expect("tab.list");
    let comments = tabs
        .as_array()
        .expect("an array")
        .iter()
        .find(|t| t["name"] == "comments")
        .expect("the comments tab");
    assert_eq!(
        comments["panes"][0].as_str(),
        Some(first_pane.as_str()),
        "the first pane of the layout has not moved"
    );
    assert_eq!(
        Path::new(comments["cwd"].as_str().expect("a cwd")),
        elsewhere,
        "the tab reports where its focused pane is, which is no longer its first"
    );
}

/// A V1 window can name a directory that has since been deleted while its workspace root
/// has not, which is not a skip because the skip asks about the root. `same_directory`
/// compares such a path as it stands rather than resolving it to nothing, so two different
/// missing directories stay two different directories.
///
/// **The name is held constant on purpose.** My first version of this test gave the two
/// windows different names, and the name alone then told the tabs apart, so replacing the
/// fallback with an empty path changed nothing and the mutant survived. Here the only
/// difference between the planned tab and the one already there is a directory that does
/// not resolve.
#[tokio::test]
async fn a_window_whose_directory_is_gone_is_told_apart_from_another_that_is_also_gone() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    let slot = f.slot.display().to_string();
    let session = |cwd: &str| {
        format!(
            r#"{{
  "name": "audrey-app-workspace-1",
  "root": "{slot}",
  "label": "reviewer",
  "windows": [{{ "index": 1, "name": "one", "cwd": "{cwd}", "agent": "" }}]
}}"#
        )
    };

    // Neither directory is ever created.
    f.write("workspace-1.json", &session(&format!("{slot}/went-away")));
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![None, Some("one".to_string())]
    );

    // The same window name, in a second directory that also does not resolve. If both
    // unresolvable paths read as one, this matches the tab that is there and is never made.
    f.write("workspace-1.json", &session(&format!("{slot}/also-gone")));
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await,
        vec![None, Some("one".to_string()), Some("one".to_string())],
        "two directories that do not resolve are still two directories"
    );
}

/// A shell that has moved makes the next import create its tab again.
///
/// This is the production behaviour of the tab identity rule, and it is provable here: the
/// harness's inspector is a test double whose directory a test can set, and the server
/// polls it on its tick. What the double cannot do is answer differently for two panes, so
/// this moves every shell at once.
#[tokio::test]
async fn a_shell_that_has_moved_makes_the_next_import_create_its_tab_again() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();
    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    let before = tab_names(&mut h, &f, "workspace-1").await;
    assert_eq!(before.len(), 3);

    // Every pane's shell reports a different directory from now on. It has to be a
    // directory no planned tab names: `frontend` would match the planned `server` tab and
    // only one of the two would be created again, which is a weaker fact than the one this
    // test is about.
    let moved = f.slot.join("moved-here");
    std::fs::create_dir_all(&moved).expect("make the directory");
    h.inspector.set(
        Some(ForegroundProcess {
            pid: 1,
            name: "sh".into(),
        }),
        Some(moved.clone()),
    );
    let workspace = workspace_id(&mut h, &f, "workspace-1").await;
    // Well clear of a loaded machine. This binary's wall clock time varies by two orders of
    // magnitude with what else is building: the same test has been measured at 0.69s and at
    // 90s on this machine, and that is true of the tests written before this one too. A
    // limit near the fast case would turn contention into a failure, and a failing test
    // reads as a killed mutant, so the number has to be an anti-hang bound rather than a
    // measurement of how long the work should take.
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let tabs = h
            .api("tab.list", json!({ "workspace": workspace }))
            .await
            .expect("tab.list");
        let all_moved = tabs
            .as_array()
            .expect("an array")
            .iter()
            .all(|t| Path::new(t["cwd"].as_str().expect("a cwd")) == moved);
        if all_moved {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the panes never reported the move"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let (out, _, stderr) = import(&h, &f, &[]).await;
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        tab_names(&mut h, &f, "workspace-1").await.len(),
        5,
        "neither planned tab matched where its shell now is, so both were made again"
    );
}

/// A server that is not listening is one sentence, not one per project.
///
/// Every `project.add` failure is reported and the import carries on, which is right for a
/// failure about that project's root. A server that is not there is not about any root and
/// gives the same answer for every project, so without a single check before the loop the
/// most likely failure the author will ever hit prints an identical line per project.
#[tokio::test]
async fn a_server_that_is_not_running_is_said_once() {
    let f = Fixture::build();
    // Three more projects, so a message per project would be four rather than one.
    for name in ["one", "two", "three"] {
        let root = f.tmp.path().join(name);
        std::fs::create_dir_all(&root).expect("make the directory");
        f.write(
            &format!("{name}.json"),
            &format!(
                r#"{{ "name": "{name}", "root": "{}", "windows": [] }}"#,
                root.display()
            ),
        );
    }

    let out = Command::new(env!("CARGO_BIN_EXE_domux"))
        .env("DOMUX_SOCKET", f.tmp.path().join("no-server.sock"))
        .env("HOME", f.home())
        .env_remove("DOMUX_TAB")
        .env_remove("DOMUX_PANE")
        .env_remove("DOMUX_WORKSPACE")
        .env_remove("TMUX")
        .args(["import", "v1", "--from"])
        .arg(f.sessions())
        .output()
        .await
        .expect("run the import");

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.matches("The server is not running").count(),
        1,
        "said once for the run, not once per project: {stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "nothing was imported, so no report: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A planned workspace the server did not register is reported and counted, and the run
/// leaves a status of 1.
///
/// The input is a project part way through V1's own rename. `git::existing_slots` reads
/// both `.domux/worktrees` and `.baag/worktrees`, and `core::slot_directory` registers the
/// slot at the `.domux` path whenever that directory is there. A V1 session still rooted at
/// the `.baag` copy therefore plans a workspace at a path the server does not register, and
/// its tabs have nowhere to go. Nothing on either side is wrong; the two answer a question
/// about the same slot with two paths.
///
/// Before this the author read one line on stderr and a status of 0, so a run that lost a
/// whole workspace looked like a success.
#[tokio::test]
async fn a_workspace_the_server_did_not_register_is_reported_and_leaves_a_failure() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_repo_tmp, root) = repo_with_origin("main");
    // Both copies on disk, which is what a part migrated project looks like.
    let current = root.join(".domux/worktrees/workspace-1");
    let legacy = root.join(".baag/worktrees/workspace-1");
    std::fs::create_dir_all(&current).expect("make the current worktree directory");
    std::fs::create_dir_all(&legacy).expect("make the legacy worktree directory");

    let tmp = tempfile::tempdir().expect("temp dir");
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).expect("make the sessions directory");
    std::fs::write(
        sessions.join("legacy.json"),
        format!(
            r#"{{ "name": "legacy-slot", "root": "{}", "label": "old slot",
                  "windows": [{{ "index": 1, "name": "one", "cwd": "{}", "agent": "" }}] }}"#,
            legacy.display(),
            legacy.display()
        ),
    )
    .expect("write a session file");

    let out = domux(&h, &tmp.path().join("home"))
        .args(["import", "v1", "--from"])
        .arg(&sessions)
        .output()
        .await
        .expect("run the import");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    assert_eq!(out.status.code(), Some(1), "a workspace was lost: {stderr}");
    assert!(
        stderr.contains(&legacy.display().to_string())
            && stderr.contains("so its tabs were not created"),
        "the workspace that did not arrive is named: {stderr}"
    );
    assert!(
        stderr.contains("Not imported: 1 workspace."),
        "and it is counted as a workspace: {stderr}"
    );
    assert_eq!(
        stdout,
        format!(
            "{root}\nImported 1 project, 0 workspaces, 0 tabs.\n",
            root = root.display()
        ),
        "the project arrived; the workspace is absent from the report and from its counts"
    );

    // The project itself is registered, with the slot at the path the server chose.
    let projects = h
        .api("project.list", json!({}))
        .await
        .expect("project.list");
    let canonical = root.canonicalize().expect("the repository is there");
    let project = projects
        .as_array()
        .expect("an array")
        .iter()
        .find(|p| Path::new(p["root"].as_str().expect("a root")) == canonical)
        .expect("the project was registered");
    let rows = h
        .api(
            "workspace.list",
            json!({ "project": project["id"].as_str().expect("an id") }),
        )
        .await
        .expect("workspace.list");
    let slot = rows
        .as_array()
        .expect("an array")
        .iter()
        .find(|w| w["handle"] == "workspace-1")
        .expect("the slot was adopted");
    assert_eq!(
        Path::new(slot["path"].as_str().expect("a path")),
        canonical.join(".domux/worktrees/workspace-1"),
        "the server registered the current path, which is not the one V1 recorded"
    );
}

/// Task 22 checks the same thing for the other M2 subcommands. `import` is checked here
/// because this is the task that creates it.
#[tokio::test]
async fn import_help_names_what_it_does() {
    for args in [
        vec!["import", "--help"],
        vec!["import", "v1", "--help"],
        vec!["--help"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_domux"))
            .env("DOMUX_SOCKET", "/nonexistent/sock")
            .args(&args)
            .output()
            .await
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.to_lowercase().contains("import"),
            "{args:?} does not name the command: {text}"
        );
        assert!(
            !text.contains('\u{2014}'),
            "help still has an em dash: {text}"
        );
        assert!(
            !text.to_lowercase().contains("to do"),
            "help still has a placeholder: {text}"
        );
    }
}
