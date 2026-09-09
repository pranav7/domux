//! End to end tests for `domux2 import v1`, run as the process a person types against a
//! harness server on a temp socket.
//!
//! Every test builds its own session files in a temp directory and passes `--from`, so
//! nothing here reads V1's real state directory. `domux2` also runs with `HOME` pointed at a
//! temp directory, so a bug that ignored `--from` would read an empty directory rather than
//! the author's own sessions.

use domux_core::config::Config;
use domux_server::testing::{repo_with_origin, Harness};
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// The CLI pointed at this harness's socket, at no pane, and at a home directory with
/// nothing in it.
fn domux2(h: &Harness, home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
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

    /// What the report says for this fixture, whichever mode the run is in.
    fn expected_report(&self) -> String {
        format!(
            "{root}\n  main, named audit-harness, tabs: agent-harness\n  workspace-1, named \
             reviewer, tabs: comments, server\nImported 1 project, 2 workspaces, 3 tabs.\n\
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
    let mut command = domux2(h, &f.home());
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
async fn a_dry_run_prints_the_same_report_and_changes_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = Fixture::build();

    let (out, dry_stdout, dry_stderr) = import(&h, &f, &["--dry-run"]).await;
    assert!(out.status.success(), "{dry_stderr}");
    assert!(
        dry_stderr.contains("Dry run: nothing was changed."),
        "{dry_stderr}"
    );
    assert_eq!(dry_stdout, f.expected_report());
    assert!(
        imported_workspaces(&mut h, &f).await.is_empty(),
        "a dry run registers nothing"
    );

    let (out, stdout, _) = import(&h, &f, &[]).await;
    assert!(out.status.success());
    assert_eq!(
        stdout, dry_stdout,
        "the run says exactly what the dry run said it would"
    );
    assert_eq!(imported_workspaces(&mut h, &f).await.len(), 2);
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
    let out = domux2(&h, &f.home())
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

    let mut command = domux2(&h, &f.home());
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

/// Task 22 checks the same thing for the other M2 subcommands. `import` is checked here
/// because this is the task that creates it.
#[tokio::test]
async fn import_help_names_what_it_does() {
    for args in [
        vec!["import", "--help"],
        vec!["import", "v1", "--help"],
        vec!["--help"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
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
