//! `domux server upgrade` end to end: a real server in its own session, a real shell in its
//! pane, and a real exec (decision 0045). Nothing here touches a real server's state directory.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::Command;

struct Scratch {
    _dir: tempfile::TempDir,
    root: PathBuf,
    socket: PathBuf,
    state: PathBuf,
    config: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let config = root.join("domux.toml");
        // A shell with no rc files, so the test does not wait on the machine's own.
        std::fs::write(&config, "[terminal]\nshell = \"/bin/sh\"\n").unwrap();
        std::fs::create_dir_all(root.join("proj")).unwrap();
        Scratch {
            socket: root.join("domux.sock"),
            state: root.join("state"),
            config,
            root,
            _dir: dir,
        }
    }

    fn cli(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_domux"));
        c.env("DOMUX_SOCKET", &self.socket)
            .env("DOMUX_STATE_DIR", &self.state)
            .env("DOMUX_CONFIG_FILE", &self.config)
            .env_remove("DOMUX_TAB")
            .env_remove("DOMUX_PANE")
            .env_remove("DOMUX_WORKSPACE")
            .env_remove("TMUX")
            .current_dir(self.root.join("proj"));
        c
    }

    async fn run(&self, args: &[&str]) -> std::process::Output {
        tokio::time::timeout(Duration::from_secs(90), self.cli().args(args).output())
            .await
            .unwrap_or_else(|_| panic!("domux {args:?} did not finish"))
            .unwrap()
    }

    async fn api(&self, method: &str, params: Value) -> Value {
        let out = self.run(&["api", method, &params.to_string()]).await;
        assert!(
            out.status.success(),
            "{method}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// The pane's text once `want` is in it, or a failure that prints what it said instead.
    async fn read_until(&self, pane: &str, want: &[&str]) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let text = self.api("pane.read", json!({"pane": pane})).await["text"]
                .as_str()
                .unwrap()
                .to_string();
            if want.iter().all(|w| text.contains(w)) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "{want:?} never appeared in:\n{text}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for Scratch {
    /// Stops the server whatever the test did, so no failure leaves one running on a socket in
    /// a deleted directory.
    fn drop(&mut self) {
        let _ = std::process::Command::new(env!("CARGO_BIN_EXE_domux"))
            .env("DOMUX_SOCKET", &self.socket)
            .env("DOMUX_STATE_DIR", &self.state)
            .env("DOMUX_CONFIG_FILE", &self.config)
            .args(["server", "stop"])
            .output();
    }
}

fn server_pid(status: &std::process::Output) -> String {
    let text = String::from_utf8_lossy(&status.stdout).to_string();
    text.split("(pid ")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .unwrap_or_else(|| panic!("no pid in {text}"))
        .to_string()
}

/// The process ids whose parent is `pid`. A pane's shell is the server's child, and after an
/// upgrade it still is.
async fn children_of(pid: &str) -> Vec<String> {
    let out = Command::new("pgrep")
        .args(["-P", pid])
        .output()
        .await
        .unwrap();
    let mut pids: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    pids.sort();
    pids
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).trim().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn server_upgrade_keeps_the_shell_in_a_pane_running_with_its_screen() {
    let s = Scratch::new();
    let started = s.run(&["server", "start"]).await;
    assert!(started.status.success(), "{}", stderr(&started));
    let workspaces = s.api("workspace.list", json!({})).await;
    let workspace = workspaces[0]["id"].as_str().unwrap().to_string();
    let tabs = s.api("tab.list", json!({"workspace": workspace})).await;
    let pane = tabs[0]["panes"][0].as_str().unwrap().to_string();
    let status = s.run(&["server", "status"]).await;
    let server = server_pid(&status);
    let shells = children_of(&server).await;
    assert_eq!(shells.len(), 1, "one shell, the pane's: {shells:?}");
    s.api(
        "pane.send_text",
        json!({"pane": pane, "text": "echo before-$((20+1))\n"}),
    )
    .await;
    s.read_until(&pane, &["before-21"]).await;

    let upgraded = s.run(&["server", "upgrade"]).await;

    assert!(upgraded.status.success(), "{}", stderr(&upgraded));
    assert!(
        stderr(&upgraded).starts_with("Upgraded the server to domux "),
        "{}",
        stderr(&upgraded)
    );
    let status = s.run(&["server", "status"]).await;
    assert_eq!(server_pid(&status), server, "the same process, replaced");
    assert!(
        String::from_utf8_lossy(&status.stdout).contains(", upgraded "),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    assert_eq!(
        children_of(&server).await,
        shells,
        "the same shell, still running"
    );
    s.read_until(&pane, &["before-21"]).await;
    s.api(
        "pane.send_text",
        json!({"pane": pane, "text": "echo after-$((40+2))\n"}),
    )
    .await;
    s.read_until(&pane, &["before-21", "after-42"]).await;
    assert!(
        !domux_core::paths::handoff_dir_under(&s.state).exists(),
        "the new server removed the handover"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn server_upgrade_with_no_server_running_starts_one() {
    let s = Scratch::new();
    let out = s.run(&["server", "upgrade"]).await;
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).starts_with("No server was running, so this started one."),
        "{}",
        stderr(&out)
    );
    let status = s.run(&["server", "status"]).await;
    assert!(status.status.success(), "{}", stderr(&status));
    assert!(Path::new(&s.socket).exists());
}

/// How many clients the server says are attached, or `None` while it does not answer.
async fn clients(s: &Scratch) -> Option<usize> {
    let out = s.run(&["api", "server.info"]).await;
    if !out.status.success() {
        return None;
    }
    let info: Value = serde_json::from_slice(&out.stdout).ok()?;
    info["clients"].as_array().map(Vec::len)
}

async fn wait_for_one_client(s: &Scratch) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if clients(s).await == Some(1) {
            return;
        }
        assert!(Instant::now() < deadline, "no client attached within 20 s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// An attached client leaves when the server upgrades and comes back by itself, from the
/// binary it was started as, without asking anything (decision 0045). The same process is still
/// on the terminal at the end, and a detach still ends it cleanly.
#[tokio::test(flavor = "multi_thread")]
async fn an_attached_client_comes_back_by_itself_after_an_upgrade() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::{Read, Write};

    let s = Scratch::new();
    let started = s.run(&["server", "start"]).await;
    assert!(started.status.success(), "{}", stderr(&started));
    let home = tempfile::tempdir().unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_domux"));
    cmd.arg("attach");
    cmd.env("HOME", home.path());
    cmd.env("TERM", "xterm-256color");
    cmd.env("DOMUX_SOCKET", &s.socket);
    cmd.env("DOMUX_STATE_DIR", &s.state);
    cmd.env("DOMUX_CONFIG_FILE", &s.config);
    for name in [
        "OMARCHY_PATH",
        "SSH_TTY",
        "SSH_CONNECTION",
        "TMUX",
        "DOMUX_PANE",
    ] {
        cmd.env_remove(name);
    }
    cmd.cwd(s.root.join("proj"));
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = output.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let said = || String::from_utf8_lossy(&output.lock().unwrap()).to_string();
    wait_for_one_client(&s).await;

    let upgraded = s.run(&["server", "upgrade"]).await;
    assert!(upgraded.status.success(), "{}", stderr(&upgraded));

    wait_for_one_client(&s).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "the client ended rather than coming back:\n{}",
        said()
    );
    assert!(
        !said().contains("Run domux to attach"),
        "the client told the reader to attach by hand:\n{}",
        said()
    );
    writer.write_all(b"\x13d").unwrap(); // C-s then d
    writer.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the client did not exit after the detach:\n{}",
            said()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(status.success(), "{status:?}\n{}", said());
}
