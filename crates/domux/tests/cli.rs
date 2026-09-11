//! End to end tests for the `domux2` binary: every subcommand is run as the process a
//! person types, against a harness server on a temp socket.
//!
//! The commands run through `tokio::process` rather than `std::process`, because the
//! harness server lives on this test's own runtime: a blocking `output()` would stop the
//! server it is waiting for.

use domux_core::config::Config;
use domux_core::model::agent::AgentKind;
use domux_server::testing::Harness;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
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

/// The text a terminal would show, with the sequences it would act on taken out. Enough
/// for these assertions, not a terminal emulator: the pane title arrives as `┌`, a bold
/// sequence, then ` sh `, and the test is about the title, not the bold.
fn visible(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then one final byte.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: a string ending in BEL or in ESC \.
            Some(']') => {
                while let Some(c) = chars.next() {
                    match c {
                        '\x07' => break,
                        '\x1b' => {
                            chars.next();
                            break;
                        }
                        _ => {}
                    }
                }
            }
            // Everything else here is ESC and one more byte.
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn api_server_info_prints_json_and_unknown_methods_fail_on_stderr() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let out = domux2(&h)
        .args(["api", "server.info"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], domux_core::VERSION);
    let out = domux2(&h)
        .args(["api", "pane.explode"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("not_found: method pane.explode does not exist"));
}

/// The schema describes this build, so it answers with nothing listening.
#[tokio::test]
async fn api_schema_needs_no_server() {
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", "/nonexistent/sock")
        .args(["api", "schema"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["methods"]["pane.split"]["params"].is_object());
}

#[tokio::test]
async fn tab_name_reads_its_tab_from_the_environment() {
    let mut h = Harness::start(Config::default(), 60, 10).await;
    h.api("tab.create", serde_json::json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(10),
    )
    .await;
    let tabs = h.api("tab.list", serde_json::json!({})).await.unwrap();
    let first = tabs[0]["id"].as_str().unwrap().to_string();
    let out = domux2(&h)
        .env("DOMUX_TAB", &first)
        .args(["tab", "name", "from shell"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "quiet on success");
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("1 from shell"),
            Duration::from_secs(10),
        )
        .await;
    assert!(f.contains("│ 2 │"), "the current tab kept its number:\n{f}");
    let out = domux2(&h)
        .env("DOMUX_TAB", &first)
        .args(["tab", "clear-name"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("from shell"),
        Duration::from_secs(10),
    )
    .await;
}

#[tokio::test]
async fn pane_split_read_and_send_text_act_on_the_pane_from_the_environment() {
    let mut h = Harness::start(Config::default(), 60, 10).await;
    let pane = h.focused_pane(h.client.clone());
    let out = domux2(&h)
        .env("DOMUX_PANE", pane.as_str())
        .args(["pane", "send-text", "echo hi"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    h.frame(h.client.clone()).await;
    assert_eq!(h.pane_input(&pane), b"echo hi".to_vec());
    // Three lines and a read of two, so the answer tells `--lines 2` from the default: on a
    // pane holding only two, both answers are the same text and the flag is unheld.
    h.feed_pane(pane.clone(), b"alpha\r\nbeta\r\ngamma").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("gamma"),
        Duration::from_secs(10),
    )
    .await;
    let out = domux2(&h)
        .env("DOMUX_PANE", pane.as_str())
        .args(["pane", "read", "--lines", "2"])
        .output()
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "beta\ngamma\n");
    // No DOMUX_PANE: the split lands on the view's focused pane instead.
    let out = domux2(&h)
        .args(["pane", "split", "down"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    h.wait_for(
        h.client.clone(),
        |f| f.matches('┌').count() == 2,
        Duration::from_secs(10),
    )
    .await;
}

#[tokio::test]
async fn server_status_reports_a_running_server_and_a_stopped_one() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let out = domux2(&h)
        .args(["server", "status"])
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
        text.starts_with(&format!("Server domux {} (pid ", domux_core::VERSION)),
        "{text}"
    );
    assert!(text.contains("Clients: 1"), "{text}");
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", "/nonexistent/sock")
        .args(["server", "status"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "The server is not running. Start it with domux2 server start."
    );
}

/// `events` is the one subcommand that keeps its connection. The subscription has to be
/// live before the event happens, and nothing tells the test when it is, so tabs are
/// created until a line arrives.
#[tokio::test]
async fn events_prints_one_json_object_per_line() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let mut child = domux2(&h)
        .args(["events", "tab.*"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let deadline = Instant::now() + Duration::from_secs(10);
    let line = loop {
        assert!(
            Instant::now() < deadline,
            "no event line within 10 s; is the subscription live?"
        );
        h.api("tab.create", serde_json::json!({})).await.unwrap();
        if let Ok(Ok(Some(line))) =
            tokio::time::timeout(Duration::from_millis(300), lines.next_line()).await
        {
            break line;
        }
    };
    let event: serde_json::Value =
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
    assert_eq!(event["event"], "tab.created", "{line}");
    assert!(event["tab"].is_string(), "{line}");
    child.kill().await.unwrap();
}

/// The whole server lifecycle, with a real `domux2 server run` in its own session and its
/// own state directory. Nothing here touches the state directory of a real server.
#[tokio::test]
async fn server_start_status_and_stop_manage_a_real_server() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("domux2.sock");
    let state = dir.path().join("state");
    let config = dir.path().join("domux.toml");
    let cli = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
        c.env("DOMUX_SOCKET", &socket)
            .env("DOMUX_STATE_DIR", &state)
            .env("DOMUX_CONFIG_FILE", &config)
            .env_remove("DOMUX_TAB")
            .env_remove("DOMUX_PANE")
            .env_remove("DOMUX_WORKSPACE")
            .env_remove("TMUX");
        c
    };
    // Every step runs before any assertion, so a failed expectation still leaves the
    // server stopped rather than running on a socket in a deleted temp directory.
    let started = cli().args(["server", "start"]).output().await.unwrap();
    let again = cli().args(["server", "start"]).output().await.unwrap();
    let status = cli().args(["server", "status"]).output().await.unwrap();
    // A first boot has no domux.toml at all. The file is written between the two reports, so
    // the same server answers for both a config that is not there and one that is.
    std::fs::write(&config, "[terminal]\nscrollback = 1000\n").unwrap();
    let with_config = cli().args(["server", "status"]).output().await.unwrap();
    // Asked while the server is still up: a session of its own is what keeps it alive when
    // the terminal that started it goes away, and a session leader's id is its own pid.
    let pid: i32 = pid_of(&status).parse().unwrap();
    let session = unsafe { libc::getsid(pid) };
    let log = cli().args(["server", "log"]).output().await.unwrap();
    let stopped = cli().args(["server", "stop"]).output().await.unwrap();
    let stopped_again = cli().args(["server", "stop"]).output().await.unwrap();

    let say = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).trim().to_string();
    assert!(started.status.success(), "{}", say(&started));
    assert!(
        say(&started).starts_with("Server started (pid "),
        "{}",
        say(&started)
    );
    assert!(started.stdout.is_empty(), "a message is not data");
    assert_eq!(say(&again), "The server is already running.");
    assert!(status.status.success(), "{}", say(&status));
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(
        text.starts_with(&format!("Server domux {} (pid ", domux_core::VERSION)),
        "{text}"
    );
    assert!(text.contains("Clients: 0"), "{text}");
    assert_eq!(
        session, pid,
        "the server did not leave this terminal's session"
    );
    assert!(
        text.contains(&format!("Config  {} (not created yet)", config.display())),
        "a config file that is not there must not read as one that is:\n{text}"
    );
    assert!(
        text.lines().any(|l| l == "Leader  C-a"),
        "the report says which key starts a chord:\n{text}"
    );
    let text = String::from_utf8_lossy(&with_config.stdout);
    assert!(
        text.lines()
            .any(|l| l == format!("Config  {}", config.display())),
        "a config file that is there is named and nothing more:\n{text}"
    );
    assert!(
        text.contains(&format!("Socket  {}", socket.display())),
        "{text}"
    );
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        state.join("server.log").display().to_string()
    );
    assert_eq!(say(&stopped), "Server stopped.");
    assert_eq!(say(&stopped_again), "The server is not running.");
    assert!(
        state.join("state.json").exists(),
        "the stop persisted the structure"
    );
    assert!(!socket.exists(), "the stop took the socket with it");
}

/// The config file and the running server can disagree about the leader: the file is read at
/// start and on `config.reload`, so an edit made after the server started is not in force. The
/// report has to answer for the server, because that is the key that works.
///
/// This is a real trap and not a theoretical one. A leader set in a file the running server
/// had never read left no way to find the leader that did work: the help overlay is the one
/// place that lists it, and reaching the help overlay needs the leader.
#[tokio::test]
async fn server_status_reports_the_leader_in_force_not_the_one_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("domux2.sock");
    let state = dir.path().join("state");
    let config = dir.path().join("domux.toml");
    let cli = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
        c.env("DOMUX_SOCKET", &socket)
            .env("DOMUX_STATE_DIR", &state)
            .env("DOMUX_CONFIG_FILE", &config)
            .env_remove("DOMUX_TAB")
            .env_remove("DOMUX_PANE")
            .env_remove("DOMUX_WORKSPACE")
            .env_remove("TMUX");
        c
    };
    let leader_line = |o: &std::process::Output| {
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .find(|l| l.starts_with("Leader"))
            .unwrap_or("(no leader line)")
            .to_string()
    };

    // Every step runs before any assertion, so a failed one still stops the server.
    cli().args(["server", "start"]).output().await.unwrap();
    // Written after the server started, which is the whole point: it is on disk and not yet
    // in force.
    std::fs::write(&config, "[keys]\nleader = \"C-s\"\n").unwrap();
    let before = cli().args(["server", "status"]).output().await.unwrap();
    cli().args(["config", "reload"]).output().await.unwrap();
    let after = cli().args(["server", "status"]).output().await.unwrap();
    let stopped = cli().args(["server", "stop"]).output().await.unwrap();

    assert_eq!(
        leader_line(&before),
        "Leader  C-a",
        "an edit the server has not read is not the leader in force"
    );
    assert_eq!(
        leader_line(&after),
        "Leader  C-s",
        "the reload put it in force"
    );
    assert!(stopped.status.success());
}

/// Collects what the pty has written until `wants` shows, or gives up after 20 seconds.
async fn wait_for_text(rx: &std::sync::mpsc::Receiver<Vec<u8>>, output: &mut Vec<u8>, wants: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        while let Ok(chunk) = rx.try_recv() {
            output.extend(chunk);
        }
        if visible(output).contains(wants) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no screen showing {wants:?} within 20 s:\n{}",
            visible(output)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Runs the client on a real pty, waits for `wants` to show on the screen, sends leader d and
/// waits for it to exit. Hands back everything the client wrote and how it ended.
/// Principle 13: the client's own terminal handling is only proved on a real terminal.
async fn attach_and_detach_in_a_pty(
    cmd: CommandBuilder,
    wants: &str,
) -> (Vec<u8>, portable_pty::ExitStatus) {
    answer_then_attach_and_detach_in_a_pty(cmd, &[], wants).await
}

/// The same, with questions answered on the way in: each pair waits for its text to show and
/// then types its answer. `attach` asks one before it attaches, so a test about that question
/// needs to answer it before there is a screen to wait for.
async fn answer_then_attach_and_detach_in_a_pty(
    cmd: CommandBuilder,
    answers: &[(&str, &str)],
    wants: &str,
) -> (Vec<u8>, portable_pty::ExitStatus) {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 12,
            cols: 50,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    // The pty only reads on a blocking thread; the test itself never blocks, so a harness
    // server on this runtime keeps drawing.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut output = Vec::new();
    for (question, answer) in answers {
        wait_for_text(&rx, &mut output, question).await;
        writer.write_all(answer.as_bytes()).unwrap();
        writer.flush().unwrap();
    }
    wait_for_text(&rx, &mut output, wants).await;
    writer.write_all(b"\x01d").unwrap(); // C-a then d
    writer.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        while let Ok(chunk) = rx.try_recv() {
            output.extend(chunk);
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the client did not exit after the detach:\n{}",
            visible(&output)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // What the client wrote last reaches the pty after it exits.
    let drain = Instant::now() + Duration::from_secs(2);
    loop {
        match rx.try_recv() {
            Ok(chunk) => output.extend(chunk),
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {
                if Instant::now() >= drain {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    (output, status)
}

#[tokio::test]
async fn attach_inside_a_pty_draws_the_screen_and_leader_d_detaches_cleanly() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_domux2"));
    cmd.arg("attach");
    cmd.env("DOMUX_SOCKET", h.socket_path());
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("TMUX");
    // In the project the server is holding, so the attach is the only thing under test.
    // `CommandBuilder` starts in the user's home directory when it is given none, and an
    // attach from a directory the server does not hold asks whether to register it.
    cmd.cwd(h.project_root());
    let (output, status) = attach_and_detach_in_a_pty(cmd, "\u{250c} sh").await;
    assert!(status.success(), "{status:?}");
    // The lifecycle is in the sequences themselves, so those are read raw.
    let raw = String::from_utf8_lossy(&output);
    assert!(raw.contains("\x1b[?1049l"), "the alternate screen was left");
    assert!(raw.contains("\x1b[?2004l"), "bracketed paste was disabled");
    let text = visible(&output);
    assert!(
        text.trim_end()
            .ends_with("Detached. Run domux2 to reattach."),
        "{text}"
    );
}

/// The task's own title, both halves: bare `domux2` attaches, and starts the server when the
/// socket is absent. Nothing points this one at a harness - the socket is where a real run
/// looks for it, and the server that answers is a real `server run` this command started.
#[tokio::test]
async fn bare_domux2_starts_the_server_when_the_socket_is_absent_and_attaches_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("run");
    let state = dir.path().join("state");
    let config = dir.path().join("domux.toml");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(&config, "[terminal]\nshell = \"/bin/sh\"\n").unwrap();
    let socket = run.join("domux2.sock");
    assert!(!socket.exists(), "nothing is listening yet");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_domux2"));
    cmd.env("XDG_RUNTIME_DIR", &run);
    cmd.env("DOMUX_STATE_DIR", &state);
    cmd.env("DOMUX_CONFIG_FILE", &config);
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("DOMUX_SOCKET");
    cmd.env_remove("TMUX");
    // A real shell's name is the machine's business (/bin/sh is bash on macOS and dash on
    // most Linux), so this waits for the pane box rather than for a title.
    let (output, status) = attach_and_detach_in_a_pty(cmd, "\u{250c}").await;
    // The server outlives the client, so it is stopped before any assertion can fail and
    // leave it running on a socket in a deleted temp directory.
    let stopped = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("XDG_RUNTIME_DIR", &run)
        .env("DOMUX_STATE_DIR", &state)
        .env("DOMUX_CONFIG_FILE", &config)
        .env_remove("DOMUX_SOCKET")
        .env_remove("TMUX")
        .args(["server", "stop"])
        .output()
        .await
        .unwrap();
    assert!(status.success(), "{status:?}");
    let text = visible(&output);
    assert!(
        text.trim_end()
            .ends_with("Detached. Run domux2 to reattach."),
        "{text}"
    );
    // The start behind an attach is silent: "Attach with domux2" would name an action already
    // underway, and this is the first command a new user types.
    assert!(
        !text.contains("Server started"),
        "the cold start named an action already underway:\n{text}"
    );
    assert_eq!(
        String::from_utf8_lossy(&stopped.stderr).trim(),
        "Server stopped.",
        "bare domux2 left a server running for the next attach"
    );
}

/// A start that fails must say why. The child's stderr is the log, so the reason survives
/// even when the server failed before its own logging was up.
#[tokio::test]
async fn server_start_says_why_the_server_could_not_start() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    // A file where a directory would have to be: the server cannot make the socket's parent,
    // and the failure is inside `server run` rather than in this process.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", blocker.join("sub").join("domux2.sock"))
        .env("DOMUX_STATE_DIR", &state)
        .env("DOMUX_CONFIG_FILE", dir.path().join("domux.toml"))
        .env_remove("TMUX")
        .args(["server", "start"])
        .output()
        .await
        .unwrap();
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    assert_eq!(out.status.code(), Some(1), "{said}");
    assert!(said.starts_with("The server exited ("), "{said}");
    assert!(said.contains("It said: "), "{said}");
    assert!(said.contains("Not a directory"), "{said}");
    let log = std::fs::read_to_string(state.join("server.log")).expect("the log was written");
    // Two lines, and each holds one half of the fix. Tracing's carries a timestamp and a
    // level, so the server logged its own failure; the bare one is the child's stderr as the
    // child wrote it, which is the half that survives a failure before the server's logging
    // is up, a panic, and anything the runtime prints.
    assert!(
        log.lines()
            .any(|l| l.contains("ERROR") && l.contains("Not a directory")),
        "the server logged its own failure:\n{log}"
    );
    assert!(
        log.lines()
            .any(|l| l.starts_with("create ") && l.contains("Not a directory")),
        "the child's stderr is the log:\n{log}"
    );
}

/// A control server that answers one call with `reply` and hands back the request the CLI
/// sent, so a test can read the method and the params rather than only the effect. The CLI
/// probes a socket with a connect and a drop before every call, so a connection that carries
/// no request is skipped rather than answered.
fn one_call(socket: &Path, reply: serde_json::Value) -> tokio::task::JoinHandle<serde_json::Value> {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut line = String::new();
            if tokio::io::BufReader::new(r)
                .read_line(&mut line)
                .await
                .unwrap_or(0)
                == 0
            {
                continue;
            }
            let response = serde_json::json!({ "id": 1, "result": reply });
            let _ = w.write_all(format!("{response}\n").as_bytes()).await;
            return serde_json::from_str(&line).unwrap();
        }
    })
}

/// The one case the liveness probe cannot rule out: a server that accepts the connection and
/// answers nothing. Every subcommand used to print a bare "EOF while parsing a value at line
/// 1 column 0", which names no state, no object and no next action.
#[tokio::test]
async fn a_server_that_answers_nothing_names_the_method_the_socket_and_the_next_action() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("domux2.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let closer = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", &socket)
        .env_remove("TMUX")
        .args(["server", "status"])
        .output()
        .await
        .unwrap();
    closer.abort();
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    assert_eq!(out.status.code(), Some(1), "{said}");
    assert!(
        said.starts_with(&format!(
            "The server did not answer server.info on {}. Run domux2 server status.: ",
            socket.display()
        )),
        "{said}"
    );
    // What follows the context is the cause. Printing only the outermost error would have
    // dropped it, and with it the half of the sentence that says what actually failed.
    assert!(
        said.split("server status.: ")
            .nth(1)
            .is_some_and(|c| !c.is_empty()),
        "{said}"
    );
}

/// `events | head` is the normal way to read a stream. A reader that stops reading is the end
/// of the output, not a failure: the write returns a broken pipe, and the CLI stops rather
/// than panicking with a status of 101.
#[tokio::test]
async fn events_stops_quietly_when_the_reader_goes_away() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("domux2.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    // A server that keeps sending, so the CLI keeps writing into the pipe the test closed.
    let streamer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut line = String::new();
        if tokio::io::BufReader::new(r)
            .read_line(&mut line)
            .await
            .unwrap_or(0)
            == 0
        {
            // The liveness probe. The call itself is the next connection.
            let (stream, _) = listener.accept().await.unwrap();
            let (r, w2) = stream.into_split();
            let mut line = String::new();
            tokio::io::BufReader::new(r)
                .read_line(&mut line)
                .await
                .unwrap();
            w = w2;
        }
        if w.write_all(b"{\"id\":1,\"result\":{}}\n").await.is_err() {
            return;
        }
        loop {
            if w.write_all(b"{\"event\":\"tab.created\",\"tab\":\"t_1\"}\n")
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", &socket)
        .env_remove("TMUX")
        .arg("events")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let first = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
        .await
        .expect("an event line within 10 s")
        .unwrap()
        .expect("an event line");
    assert!(first.contains("tab.created"), "{first}");
    drop(lines); // the reader goes away, as `head` does after its last line
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("the CLI ended when the reader did")
        .unwrap();
    streamer.abort();
    assert!(
        status.success(),
        "a reader that stopped reading is not a failure: {status:?}"
    );
}

/// The pid out of a `server status` line, which is how a test tells one server from the next.
fn pid_of(status: &std::process::Output) -> String {
    let text = String::from_utf8_lossy(&status.stdout).to_string();
    text.split("(pid ")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .unwrap_or_else(|| panic!("no pid in {text}"))
        .to_string()
}

/// `restart` is a stop and a start, and on a stopped server it is a start. It used to say
/// "The server is not running." and then "Server started", which is true and reads as a
/// contradiction.
#[tokio::test]
async fn server_restart_replaces_a_running_server_and_starts_a_stopped_one() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("domux2.sock");
    let state = dir.path().join("state");
    let cli = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
        c.env("DOMUX_SOCKET", &socket)
            .env("DOMUX_STATE_DIR", &state)
            .env("DOMUX_CONFIG_FILE", dir.path().join("domux.toml"))
            .env_remove("DOMUX_TAB")
            .env_remove("DOMUX_PANE")
            .env_remove("DOMUX_WORKSPACE")
            .env_remove("TMUX");
        c
    };
    // Every step runs before any assertion, so a failed expectation still leaves the server
    // stopped rather than running on a socket in a deleted temp directory.
    let cold = cli().args(["server", "restart"]).output().await.unwrap();
    let first = cli().args(["server", "status"]).output().await.unwrap();
    let again = cli().args(["server", "restart"]).output().await.unwrap();
    let second = cli().args(["server", "status"]).output().await.unwrap();
    let stopped = cli().args(["server", "stop"]).output().await.unwrap();

    let say = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).trim().to_string();
    assert!(cold.status.success(), "{}", say(&cold));
    assert!(
        say(&cold).starts_with("Server started (pid "),
        "a restart with nothing running is a start: {}",
        say(&cold)
    );
    assert!(
        !say(&cold).contains("not running"),
        "the stop is quiet about a server that was not there: {}",
        say(&cold)
    );
    assert!(first.status.success(), "{}", say(&first));
    assert!(again.status.success(), "{}", say(&again));
    assert_eq!(
        say(&again).lines().next(),
        Some("Server stopped."),
        "{}",
        say(&again)
    );
    assert!(
        second.status.success(),
        "the restart started one: {}",
        say(&second)
    );
    assert_ne!(pid_of(&first), pid_of(&second), "the restart replaced it");
    assert_eq!(say(&stopped), "Server stopped.");
}

/// The ruling on the nested attach: bare `domux2` inside a pane refuses rather than drawing a
/// second whole screen inside one pane of the screen it is drawing. Both halves are pinned,
/// because the subcommands are the reason those variables are exported in the first place.
///
/// A pane is `DOMUX_PANE`, not `DOMUX_SOCKET`. The server exports both, but `DOMUX_SOCKET` is
/// also the documented way to point the CLI at a scratch server beside the real one, so a
/// guard that read it refused every attach for anyone who exports it - and said they were in a
/// pane when they were not. See the case below.
#[tokio::test]
async fn bare_domux2_inside_a_pane_refuses_while_its_subcommands_still_work() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let refused = domux2(&h)
        .env("DOMUX_PANE", "p_0001")
        .output()
        .await
        .unwrap();
    let said = String::from_utf8_lossy(&refused.stderr).trim().to_string();
    assert_eq!(refused.status.code(), Some(1), "{said}");
    assert!(
        said.starts_with("domux2 is already running in this terminal."),
        "{said}"
    );
    assert!(said.contains("domux2 tab create"), "{said}");
    assert!(refused.stdout.is_empty(), "a refusal is not data");
    let worked = domux2(&h).args(["tab", "create"]).output().await.unwrap();
    assert!(
        worked.status.success(),
        "a subcommand in a pane is unaffected: {}",
        String::from_utf8_lossy(&worked.stderr)
    );
}

/// `api schema` answers for this build, so params cannot mean anything to it. Printing the
/// schema and saying nothing would read as though they had been used.
#[tokio::test]
async fn api_schema_refuses_params_rather_than_ignoring_them() {
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", "/nonexistent/sock")
        .args(["api", "schema", "{\"lines\":2}"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "the schema was not printed anyway");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "domux2 api schema takes no params. Run it with no argument."
    );
}

/// An empty answer and one blank line are not the same fact.
#[tokio::test]
async fn pane_read_prints_nothing_for_a_pane_with_nothing_on_it() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    let out = domux2(&h)
        .env("DOMUX_PANE", pane.as_str())
        .args(["pane", "read"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Runs one subcommand against a server that answers `reply`, and hands back the request it
/// sent. Every location variable is cleared first, so a case that wants one sets it itself.
async fn request_for(
    socket: &Path,
    env: &[(&str, &str)],
    args: &[&str],
    reply: serde_json::Value,
) -> serde_json::Value {
    let server = one_call(socket, reply);
    let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
    c.env("DOMUX_SOCKET", socket)
        .env_remove("DOMUX_TAB")
        .env_remove("DOMUX_PANE")
        .env_remove("DOMUX_WORKSPACE")
        .env_remove("TMUX");
    for (name, value) in env {
        c.env(name, value);
    }
    let out = c.args(args).output().await.unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap_or_else(|_| panic!("{args:?} never called the server"))
        .unwrap()
}

/// One subcommand, the shell it is typed in, and the call it must make.
struct Case {
    args: Vec<&'static str>,
    env: &'static [(&'static str, &'static str)],
    method: &'static str,
    params: serde_json::Value,
    /// What the server answers. An empty object is enough for every subcommand that reads
    /// nothing back; the two that read a typed result say so with `answered`.
    reply: serde_json::Value,
}

fn case(
    args: &[&'static str],
    env: &'static [(&'static str, &'static str)],
    method: &'static str,
    params: serde_json::Value,
) -> Case {
    Case {
        args: args.to_vec(),
        env,
        method,
        params,
        reply: serde_json::json!({}),
    }
}

impl Case {
    fn answered(mut self, reply: serde_json::Value) -> Case {
        self.reply = reply;
        self
    }
}

/// One whole `AgentInfo`, for the case that reads a typed result back. Spelled out rather than
/// built from the model, because what is under test is that this CLI can read the answer the
/// wire carries.
fn an_agent() -> serde_json::Value {
    serde_json::json!({
        "id": "a_5e21",
        "kind": "claude",
        "name": null,
        "session_id": "s1",
        "state": "idle",
        "unseen": false,
        "recap": null,
        "reason": null,
        "cwd": "/tmp",
        "pane": "p_1",
        "workspace": "w_1",
        "project": null,
        "tab": null,
        "place": "audrey-app \u{203a} main",
        "started_at": "2026-09-04T14:32:00+00:00",
        "last_activity_at": "2026-09-04T14:32:00+00:00",
    })
}

/// A shell inside a pane, and one outside every tab.
const IN_A_TAB: &[(&str, &str)] = &[("DOMUX_TAB", "t_1"), ("DOMUX_WORKSPACE", "w_1")];
const IN_A_PANE: &[(&str, &str)] = &[("DOMUX_PANE", "p_1")];
const ANYWHERE: &[(&str, &str)] = &[];

/// Every subcommand is one API call, and the method name and the params are the whole of what
/// it sends. A typo in one of these literals reaches a user as `not_found` at a terminal, so
/// each one is read off the wire here rather than trusted.
#[tokio::test]
async fn every_subcommand_sends_the_method_and_the_params_it_claims() {
    let dir = tempfile::tempdir().unwrap();
    let cases = vec![
        case(
            &["tab", "create"],
            IN_A_TAB,
            "tab.create",
            serde_json::json!({ "workspace": "w_1" }),
        ),
        case(
            &["tab", "name", "release"],
            IN_A_TAB,
            "tab.rename",
            serde_json::json!({ "tab": "t_1", "name": "release" }),
        ),
        case(
            &["tab", "clear-name"],
            IN_A_TAB,
            "tab.clear_name",
            serde_json::json!({ "tab": "t_1" }),
        ),
        case(
            &["tab", "close"],
            IN_A_TAB,
            "tab.close",
            serde_json::json!({ "tab": "t_1" }),
        ),
        case(
            &["tab", "select", "2"],
            IN_A_TAB,
            "tab.select",
            serde_json::json!({ "tab": "2" }),
        ),
        case(
            &["pane", "split", "left"],
            IN_A_PANE,
            "pane.split",
            serde_json::json!({ "pane": "p_1", "dir": "left" }),
        ),
        case(
            &["pane", "close"],
            IN_A_PANE,
            "pane.close",
            serde_json::json!({ "pane": "p_1" }),
        ),
        case(
            &["pane", "focus"],
            IN_A_PANE,
            "pane.focus",
            serde_json::json!({ "pane": "p_1" }),
        ),
        // The argument names a pane other than this one, and it wins over the environment.
        case(
            &["pane", "focus", "p_9"],
            IN_A_PANE,
            "pane.focus",
            serde_json::json!({ "pane": "p_9" }),
        ),
        case(
            &["pane", "zoom"],
            IN_A_PANE,
            "pane.zoom",
            serde_json::json!({ "pane": "p_1" }),
        ),
        case(
            &["pane", "read", "--lines", "7"],
            IN_A_PANE,
            "pane.read",
            serde_json::json!({ "pane": "p_1", "lines": 7 }),
        )
        .answered(serde_json::json!({ "text": "" })),
        case(
            &["pane", "send-text", "echo hi"],
            IN_A_PANE,
            "pane.send_text",
            serde_json::json!({ "pane": "p_1", "text": "echo hi" }),
        ),
        case(
            &["pane", "send-key", "C-c"],
            IN_A_PANE,
            "pane.send_key",
            serde_json::json!({ "pane": "p_1", "key": "C-c" }),
        ),
        case(
            &["config", "reload"],
            ANYWHERE,
            "config.reload",
            serde_json::json!({}),
        )
        .answered(serde_json::json!({ "error": null, "warnings": [] })),
        case(
            &["events", "tab.*", "pane.split"],
            ANYWHERE,
            "events.subscribe",
            serde_json::json!({ "filter": ["tab.*", "pane.split"] }),
        ),
        case(
            &["api", "focus.left"],
            ANYWHERE,
            "focus.left",
            serde_json::json!({}),
        ),
        case(
            &["api", "pane.resize", "{\"pane\":\"p_2\",\"delta\":3}"],
            ANYWHERE,
            "pane.resize",
            serde_json::json!({ "pane": "p_2", "delta": 3 }),
        ),
        case(
            &["server", "stop"],
            ANYWHERE,
            "server.stop",
            serde_json::json!({}),
        ),
        // M3. `peek` and `agent list` are two spellings of one call; the block the context
        // names is the short one, and the namespaced one keeps the namespace whole.
        case(&["peek"], ANYWHERE, "agent.list", serde_json::json!({}))
            .answered(serde_json::json!({ "agents": [], "red_dots": 0 })),
        case(
            &["agent", "list", "--json"],
            ANYWHERE,
            "agent.list",
            serde_json::json!({}),
        )
        .answered(serde_json::json!({ "agents": [], "red_dots": 0 })),
        case(
            &["whoami"],
            IN_A_PANE,
            "agent.self",
            serde_json::json!({ "pane": "p_1" }),
        )
        .answered(an_agent()),
        case(
            &["agent", "focus", "a_5e21"],
            ANYWHERE,
            "agent.focus",
            serde_json::json!({ "agent": "a_5e21" }),
        ),
        case(
            &["send", "a_5e21", "ping"],
            ANYWHERE,
            "agent.send",
            serde_json::json!({ "agent": "a_5e21", "text": "ping" }),
        ),
        case(
            &["read", "a_5e21"],
            ANYWHERE,
            "agent.read",
            serde_json::json!({ "agent": "a_5e21" }),
        ),
        case(
            &["wait", "a_5e21", "--timeout-ms", "500"],
            ANYWHERE,
            "agent.wait",
            serde_json::json!({ "agent": "a_5e21", "timeout_ms": 500 }),
        ),
    ];

    for (n, c) in cases.into_iter().enumerate() {
        let socket = dir.path().join(format!("s{n}.sock"));
        let request = request_for(&socket, c.env, &c.args, c.reply).await;
        assert_eq!(request["method"], c.method, "{:?}", c.args);
        assert_eq!(request["params"], c.params, "{:?}", c.args);
    }
}

/// A tab command outside a pane acts on the view's own tab, which is what the server reads a
/// null `tab` as. An absent variable must arrive as null rather than as a name no object has.
#[tokio::test]
async fn a_shell_outside_a_pane_sends_no_location_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let request = request_for(
        &dir.path().join("s.sock"),
        &[("DOMUX_TAB", ""), ("DOMUX_WORKSPACE", "")],
        &["tab", "create"],
        serde_json::json!({}),
    )
    .await;
    assert_eq!(request["params"], serde_json::json!({ "workspace": null }));
    let request = request_for(
        &dir.path().join("s2.sock"),
        &[],
        &["tab", "close"],
        serde_json::json!({}),
    )
    .await;
    assert_eq!(request["params"], serde_json::json!({ "tab": null }));
}

/// A reload whose file did not load is a failure of the reload, not a cheerful line over a
/// config that never applied. Warnings are the server's own, and are not this CLI's to drop.
#[tokio::test]
async fn config_reload_reports_warnings_and_fails_when_the_file_did_not_load() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("s.sock");
    let server = one_call(
        &socket,
        serde_json::json!({
            "error": "domux.toml line 3: unknown key clock",
            "warnings": ["unknown table [worktrees] (line 1) is ignored"],
        }),
    );
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", &socket)
        .env_remove("TMUX")
        .args(["config", "reload"])
        .output()
        .await
        .unwrap();
    server.await.unwrap();
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(1), "{said}");
    assert!(
        said.contains("warning: unknown table [worktrees] (line 1) is ignored"),
        "{said}"
    );
    assert!(
        said.contains("domux.toml line 3: unknown key clock"),
        "{said}"
    );
    assert!(
        !said.contains("Config reloaded."),
        "a config that did not apply was not reloaded: {said}"
    );
    assert!(out.stdout.is_empty(), "a message is not data");

    let socket = dir.path().join("s2.sock");
    let server = one_call(
        &socket,
        serde_json::json!({ "error": null, "warnings": [] }),
    );
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", &socket)
        .env_remove("TMUX")
        .args(["config", "reload"])
        .output()
        .await
        .unwrap();
    server.await.unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "a message is not data");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "Config reloaded."
    );
}

/// `events` needs a server, and says the one thing every subcommand says when there is none.
#[tokio::test]
async fn events_without_a_server_says_the_server_is_not_running() {
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", "/nonexistent/sock")
        .env_remove("TMUX")
        .args(["events", "tab.*"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "The server is not running. Start it with domux2 server start."
    );
}

/// `DOMUX_SOCKET` on its own is not a pane. It is the documented way to point the CLI at a
/// scratch server beside the real one, so a guard that read it told anyone who exports it that
/// they were inside a pane - which was not true - and left them no way to attach at all.
///
/// The attach here cannot succeed: there is no terminal on this process's stdout, so it ends
/// with the terminal's own error. What is pinned is that it got that far, rather than being
/// refused for a state the shell is not in.
#[tokio::test]
async fn the_socket_override_alone_is_not_a_pane() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let out = domux2(&h).output().await.unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        !said.contains("already running in this terminal"),
        "refused an attach outside a pane: {said}"
    );
}

// ---------------------------------------------------------------------------
// M3: agent, peek, whoami, resume, install
// ---------------------------------------------------------------------------

/// The agent report subcommand the way a hook runs it: the payload on standard input.
///
/// `output()` gives a child a null standard input, so a report run that way reads an empty
/// payload and pins nothing. Every report here is spawned with a pipe and the payload written
/// into it.
async fn report_through_the_cli(cmd: &mut Command, payload: &str) -> std::process::Output {
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(payload.as_bytes()).await.unwrap();
    drop(stdin);
    child.wait_with_output().await.unwrap()
}

/// The same hook lines run under V1's tmux panes until the cut-over, so a report from a shell
/// that is not in a domux pane exits 0 and says nothing (M3 plan assumption 17).
///
/// The socket here is live and would answer. What is pinned is that nothing was posted to it at
/// all: a report that sent a null pane would be refused by the server and would look exactly
/// the same from outside, and silence alone is also what a subcommand that does nothing in
/// every case would produce.
#[tokio::test]
async fn agent_report_outside_a_domux_pane_exits_zero_and_says_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("s.sock");
    let server = one_call(&socket, serde_json::json!({}));
    let out = report_through_the_cli(
        Command::new(env!("CARGO_BIN_EXE_domux2"))
            .env("DOMUX_SOCKET", &socket)
            .env_remove("DOMUX_PANE")
            .env_remove("TMUX")
            .args(["agent", "report", "--agent", "claude"]),
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    // The process has already exited, so a call it made would have been answered by now.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), server)
            .await
            .is_err(),
        "a report outside a pane called the server anyway"
    );
}

/// A hook that fails is an interruption in the agent's session, so a report to a socket nothing
/// is listening on exits 0 in silence too (M3 plan assumption 17). Every other subcommand says
/// "The server is not running"; this one is the exception, and it is the exception on purpose.
#[tokio::test]
async fn agent_report_without_a_server_exits_zero_and_says_nothing() {
    let out = report_through_the_cli(
        Command::new(env!("CARGO_BIN_EXE_domux2"))
            .env("DOMUX_SOCKET", "/nonexistent/sock")
            .env("DOMUX_PANE", "p_0001")
            .env_remove("TMUX")
            .args(["agent", "report", "--agent", "claude"]),
        r#"{"hook_event_name":"Stop","session_id":"s1"}"#,
    )
    .await;
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

/// Claude's whole hook path end to end: the payload reaches `agent.report`, the record it makes
/// carries the session the payload named, and the plain `SessionStart` block comes back on
/// stdout, which is where Claude Code reads a hook's context from.
#[tokio::test]
async fn claude_agent_report_prints_the_plain_context_block_on_session_start() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    let out = report_through_the_cli(
        domux2(&h)
            .env("DOMUX_PANE", pane.as_str())
            .args(["agent", "report", "--agent", "claude"]),
        r#"{"hook_event_name":"SessionStart","session_id":"s1","cwd":"/tmp"}"#,
    )
    .await;
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("[domux] You are agent a_"), "{text}");
    assert!(text.contains("domux2 peek"), "{text}");
    assert!(text.contains("domux2 whoami"), "{text}");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "", "{out:?}");

    let agents = h.agents().await;
    assert_eq!(agents.len(), 1, "{agents:?}");
    assert_eq!(agents[0].kind.as_str(), "claude");
    assert_eq!(agents[0].session_id.as_deref(), Some("s1"));

    // Only `SessionStart` carries a block, so every other event prints nothing at all: a hook
    // that echoed something on each event would put that text into the agent's context.
    let out = report_through_the_cli(
        domux2(&h)
            .env("DOMUX_PANE", pane.as_str())
            .args(["agent", "report", "--agent", "claude"]),
        r#"{"hook_event_name":"Stop","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "",
        "only SessionStart prints"
    );
}

/// The exact wire shape of a report: the pane from the environment, the kind from the flag and
/// the payload parsed from standard input, so a hook posts the object the agent wrote rather
/// than a string holding it.
#[tokio::test]
async fn agent_report_sends_the_pane_the_kind_and_the_parsed_payload() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("s.sock");
    let server = one_call(
        &socket,
        serde_json::json!({"agent": "a_5e21", "state": "idle", "context": null}),
    );
    let out = report_through_the_cli(
        Command::new(env!("CARGO_BIN_EXE_domux2"))
            .env("DOMUX_SOCKET", &socket)
            .env("DOMUX_PANE", "p_1")
            .env_remove("TMUX")
            .args(["agent", "report", "--agent", "codex"]),
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
    assert!(out.status.success(), "{out:?}");
    let request = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("agent report never called the server")
        .unwrap();
    assert_eq!(request["method"], "agent.report");
    assert_eq!(
        request["params"],
        serde_json::json!({
            "pane": "p_1",
            "kind": "codex",
            "payload": {"hook_event_name": "SessionStart", "session_id": "s1"},
        })
    );
}

/// The row grammar as text: the dot line, then the place line carrying the id, in the order
/// the Agents box draws the same records in (M3 plan assumption 38).
///
/// Two agents in two states, because "in the box's order" is a claim about more than one row.
/// The waiting one sorts ahead of the idle one (interface spec 6.7), and the assertion reads
/// the order off `agent.list` rather than restating it, so this holds whatever the order is.
#[tokio::test]
async fn peek_prints_one_agent_per_block_in_the_boxs_order() {
    let mut h = Harness::start(Config::default(), 60, 10).await;
    let first = h.focused_pane(h.client.clone());
    let split = h
        .api(
            "pane.split",
            serde_json::json!({"pane": first.as_str(), "dir": "down"}),
        )
        .await
        .unwrap();
    let second: domux_core::ids::PaneId =
        serde_json::from_value(split["id"].clone()).expect("the new pane's id");
    h.report(
        first.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
    h.report(
        second.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s2"}"#,
    )
    .await;
    h.report(
        second.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"s2","message":"needs permission"}"#,
    )
    .await;

    let out = domux2(&h).arg("peek").output().await.unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("• claude  waiting"), "{text}");
    assert!(lines[1].starts_with("  claude · "), "{text}");
    assert!(
        lines[1].contains("(a_"),
        "the id is on the place line so a caller can target it: {text}"
    );
    assert!(lines[2].starts_with("• claude  idle"), "{text}");

    // The same order the box sorts its rows in, read off the list both surfaces share.
    let order: Vec<String> = h
        .agents()
        .await
        .into_iter()
        .map(|a| a.id.to_string())
        .collect();
    assert_eq!(order.len(), 2, "two records");
    let printed: Vec<String> = lines
        .iter()
        .filter_map(|l| {
            l.rsplit_once('(')
                .map(|(_, id)| id.trim_end_matches(')').to_string())
        })
        .collect();
    assert_eq!(printed, order, "{text}");
}

/// `--json` is the API result, not a rendering of it (M3 plan assumption 38): what the
/// subcommand prints and what the method answers are compared against each other, so a field
/// dropped or renamed on the way through fails here.
#[tokio::test]
async fn peek_json_is_the_api_result_verbatim() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane,
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
    let out = domux2(&h).args(["peek", "--json"]).output().await.unwrap();
    assert!(out.status.success(), "{out:?}");
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // Not vacuous: an empty answer compared against an empty answer would pass.
    assert_eq!(printed["agents"][0]["session_id"], "s1", "{printed}");
    let answered = h.api("agent.list", serde_json::json!({})).await.unwrap();
    assert_eq!(printed, answered);
}

/// An empty list is a state with a next action (principle 9), and it is not a failure. The
/// sentence is the one both Agents boxes draw, so a reader who has seen one and then the other
/// is not told two different things.
#[tokio::test]
async fn peek_with_no_agents_says_so_on_stderr_and_exits_zero() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let out = domux2(&h).arg("peek").output().await.unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "No agents yet. Start claude or codex in a pane.\n"
    );
}

#[tokio::test]
async fn whoami_prints_this_panes_agent_and_exits_one_when_there_is_none() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    let out = domux2(&h)
        .env("DOMUX_PANE", pane.as_str())
        .arg("whoami")
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "not_found: no agent is running in this pane\n"
    );
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
    let out = domux2(&h)
        .env("DOMUX_PANE", pane.as_str())
        .arg("whoami")
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("claude"), "{text}");
    assert!(text.contains(" › "), "the place: {text}");
}

/// The question whoami answers is "which agent am I", so a shell that is not in a pane is told
/// where to run it rather than given whichever agent the keyboard is looking at.
#[tokio::test]
async fn whoami_outside_a_pane_says_where_to_run_it() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let out = domux2(&h).arg("whoami").output().await.unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "invalid_params: domux2 whoami needs a pane; run it inside a domux pane, where DOMUX_PANE is set\n"
    );
}

/// The three messaging verbs the `SessionStart` block names. They are not built until M4, and
/// what they answer says so: without them the block would name three commands clap does not
/// know, and the reader would be told the subcommand is unrecognised rather than when it
/// arrives (principle 9).
#[tokio::test]
async fn the_messaging_verbs_the_context_block_names_say_when_messaging_arrives() {
    let h = Harness::start(Config::default(), 40, 10).await;
    for (args, method) in [
        (vec!["send"], "agent.send"),
        (vec!["read"], "agent.read"),
        (vec!["wait"], "agent.wait"),
    ] {
        let out = domux2(&h).args(&args).output().await.unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            format!("unavailable: {method} arrives with messaging in M4 and is not built yet\n"),
            "{args:?}"
        );
    }
}

/// The binary with no socket at all: installing hooks needs no server.
///
/// `CLAUDE_CONFIG_DIR` is removed because an install follows it (decision record 0036), and the
/// author runs this suite from inside a Claude session that sets it. Leaving it would point
/// every test below at the author's own configuration directory.
fn install_cmd(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_domux2"));
    c.env("HOME", home).env_remove("TMUX");
    c.env_remove("DOMUX_SOCKET");
    c.env_remove("CLAUDE_CONFIG_DIR");
    c
}

/// Claude reads its settings from `CLAUDE_CONFIG_DIR` when that names a directory, and a session
/// started that way never reads `~/.claude/settings.json`. So an install follows the variable,
/// and `--dir` follows the reader over both.
#[tokio::test]
async fn install_claude_follows_the_config_dir_the_variable_names_and_the_flag_over_it() {
    let home = tempfile::tempdir().unwrap();
    let named = home.path().join(".claude-bedrock");
    let asked = home.path().join("elsewhere");

    let out = install_cmd(home.path())
        .env("CLAUDE_CONFIG_DIR", &named)
        .args(["install", "claude", "--apply"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let written = std::fs::read_to_string(named.join("settings.json")).unwrap();
    assert!(written.contains("agent report --agent claude"), "{written}");
    assert!(
        !home.path().join(".claude").exists(),
        "and nothing was written under the default directory"
    );

    let out = install_cmd(home.path())
        .env("CLAUDE_CONFIG_DIR", &named)
        .args(["install", "claude", "--apply"])
        .arg("--dir")
        .arg(&asked)
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        std::fs::read_to_string(asked.join("settings.json"))
            .unwrap()
            .contains("agent report --agent claude"),
        "the flag wins over the variable"
    );
}

#[tokio::test]
async fn install_without_apply_writes_nothing_and_prints_the_diff() {
    let home = tempfile::tempdir().unwrap();
    let out = install_cmd(home.path())
        .args(["install", "claude"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Would create"), "{text}");
    assert!(text.contains("+ SessionStart"), "{text}");
    assert!(text.contains("agent report --agent claude"), "{text}");
    assert!(
        !home.path().join(".claude/settings.json").exists(),
        "a preview writes nothing"
    );
    assert!(
        !home.path().join(".claude").exists(),
        "a preview makes no directory either"
    );
}

#[tokio::test]
async fn install_apply_writes_the_file_and_names_the_backup() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    std::fs::write(home.path().join(".claude/settings.json"), "{}\n").unwrap();
    let out = install_cmd(home.path())
        .args(["install", "claude", "--apply"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Patched"), "{text}");
    assert!(text.contains(".domux-backup-"), "{text}");
    let written = std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap();
    assert!(written.contains("agent report --agent claude"), "{written}");
    // The backup the message names is on disk and holds what was there before.
    let backup = text
        .lines()
        .find_map(|l| l.split_once("is at ").map(|(_, p)| p.trim_end_matches('.')))
        .unwrap_or_else(|| panic!("no backup path in {text}"));
    assert_eq!(std::fs::read_to_string(backup).unwrap(), "{}\n");

    // A second apply changes nothing and says so, rather than writing another backup.
    let out = install_cmd(home.path())
        .args(["install", "claude", "--apply"])
        .output()
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("Nothing to change."), "{text}");
    assert!(!text.contains(".domux-backup-"), "{text}");
}

/// A home with no settings file at all: the install creates one and says so. "Patched" would be
/// a claim about a file that was not there.
#[tokio::test]
async fn install_apply_creates_the_file_when_there_is_none() {
    let home = tempfile::tempdir().unwrap();
    let out = install_cmd(home.path())
        .args(["install", "claude", "--apply"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("Created "), "{text}");
    assert!(
        !text.contains(".domux-backup-"),
        "there was nothing to back up: {text}"
    );
    let written = std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap();
    assert!(written.contains("agent report --agent claude"), "{written}");
}

/// The command `install codex` writes is the whole boundary Codex runs: its SessionStart reply
/// must be Codex's JSON object, not the raw context block that starts with JSON's `[` byte.
#[tokio::test]
async fn install_codex_writes_a_session_start_hook_with_valid_json_output() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    let home = tempfile::tempdir().unwrap();
    let out = install_cmd(home.path())
        .args(["install", "codex", "--apply"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");

    let path = home.path().join(".codex/hooks.json");
    let installed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let command = installed["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.ends_with("agent report --agent codex"), "{command}");

    let out = report_through_the_cli(
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .env("DOMUX_SOCKET", h.socket_path())
            .env("DOMUX_PANE", pane.as_str())
            .env_remove("TMUX"),
        r#"{"hook_event_name":"SessionStart","session_id":"s1","cwd":"/tmp"}"#,
    )
    .await;
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "", "{out:?}");
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        printed["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let context = printed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(context.starts_with("[domux] You are agent a_"), "{context}");
    assert_eq!(
        printed,
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            }
        })
    );
}

/// The hook command is the symlink in `~/bin` when there is one, because it survives a rebuild
/// that moves the executable (M3 plan assumption 16). Every other install test falls through to
/// the running binary, so this is the only place the branch that runs on a real machine is taken.
#[tokio::test]
async fn install_writes_the_symlink_path_when_one_is_in_bin() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    let linked = home.path().join("bin/domux2");
    std::fs::write(&linked, "#!/bin/sh\n").unwrap();
    let out = install_cmd(home.path())
        .args(["install", "claude"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(&format!("{} agent report --agent claude", linked.display())),
        "the symlink, not the running binary: {text}"
    );
}

#[tokio::test]
async fn install_names_the_three_kinds_when_asked_for_another() {
    let home = tempfile::tempdir().unwrap();
    let out = install_cmd(home.path())
        .args(["install", "gemini"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "clap rejects an unknown value");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("claude, codex or opencode"), "{said}");
}

/// MUX-6: attach offers to register the directory it was typed in.
///
/// Decision record 0009. The offer runs before the attach, so the question and its answer are
/// on the plain terminal rather than over the screen the client is about to draw.
#[tokio::test]
async fn attach_from_an_unregistered_directory_offers_to_register_it() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let elsewhere = tempfile::tempdir().unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_domux2"));
    cmd.arg("attach");
    cmd.env("DOMUX_SOCKET", h.socket_path());
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("TMUX");
    cmd.cwd(elsewhere.path());
    let (output, status) = answer_then_attach_and_detach_in_a_pty(
        cmd,
        &[("is not a project yet. Register it? [y/N]", "y\n")],
        "\u{250c} sh",
    )
    .await;
    assert!(status.success(), "{status:?}");
    let registered = h
        .model()
        .projects
        .iter()
        .any(|p| p.root == elsewhere.path().canonicalize().unwrap());
    assert!(
        registered,
        "the directory became a project:\n{}",
        visible(&output)
    );
}

/// MUX-11: the offer works when nothing is attached, which is the state it is most often
/// typed in.
///
/// The offer runs `project.add` and then `workspace.focus`, and the focus is made before this
/// command has attached anything, so before decision record 0021 it was refused with "no
/// client is attached" and the `?` on it took the whole attach down. The reader was left with
/// a registered project, no screen, and a sentence telling them to run the command they had
/// just run.
///
/// The wait is on the top bar naming the new project, so this pins where the client landed
/// and not merely that it drew something: `dotfiles` is not the harness's own project.
#[tokio::test]
async fn attach_with_nothing_attached_registers_the_directory_and_seats_the_client_in_it() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let elsewhere = tempfile::tempdir().unwrap();
    let root = elsewhere.path().join("dotfiles");
    std::fs::create_dir(&root).unwrap();
    let client = h.client.clone();
    h.detach(client).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.model().clients.is_empty() {
        assert!(Instant::now() < deadline, "a client was still attached");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_domux2"));
    cmd.arg("attach");
    cmd.env("DOMUX_SOCKET", h.socket_path());
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("TMUX");
    cmd.cwd(&root);
    let (output, status) = answer_then_attach_and_detach_in_a_pty(
        cmd,
        &[("is not a project yet. Register it? [y/N]", "y\n")],
        "dotfiles \u{203a} main",
    )
    .await;

    assert!(status.success(), "{status:?}\n{}", visible(&output));
    assert!(
        !visible(&output).contains("no client is attached"),
        "the switch was made for the client this command was about to attach:\n{}",
        visible(&output)
    );
    let registered = h
        .model()
        .projects
        .iter()
        .any(|p| p.root == root.canonicalize().unwrap());
    assert!(
        registered,
        "the directory became a project:\n{}",
        visible(&output)
    );
}

/// The default is the answer that changes nothing, and saying no still attaches.
#[tokio::test]
async fn declining_the_offer_leaves_the_directory_alone_and_still_attaches() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let elsewhere = tempfile::tempdir().unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_domux2"));
    cmd.arg("attach");
    cmd.env("DOMUX_SOCKET", h.socket_path());
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("TMUX");
    cmd.cwd(elsewhere.path());
    let (output, status) = answer_then_attach_and_detach_in_a_pty(
        cmd,
        &[("is not a project yet. Register it? [y/N]", "\n")],
        "\u{250c} sh",
    )
    .await;
    assert!(status.success(), "{status:?}");
    assert_eq!(
        h.model().projects.len(),
        1,
        "no project was added:\n{}",
        visible(&output)
    );
    assert!(
        visible(&output).contains("Left unregistered. Run domux2 open . to register it later."),
        "it names the way to do it later:\n{}",
        visible(&output)
    );
}

/// `stay-awake` end to end: the subcommand a person types reaches the same handler the key
/// does, and says where it left the machine (MUX-15, decision 0029).
#[tokio::test]
async fn stay_awake_on_and_off_say_where_they_left_the_machine() {
    let h = Harness::start(Config::default(), 80, 24).await;
    h.runner.on_path("caffeinate");
    let out = domux2(&h)
        .args(["stay-awake", "on"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Stay awake is on.\n");
    assert!(
        h.runner.ran("caffeinate", &["-dimsu"]),
        "{:?}",
        h.runner.calls()
    );

    let out = domux2(&h)
        .args(["stay-awake", "status"])
        .output()
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Stay awake is on.\n");

    let out = domux2(&h)
        .args(["stay-awake", "off"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Stay awake is off.\n");
}

/// A machine domux cannot hold awake is not a failure of the command, but it is a failure of
/// what was asked for: the state goes to stdout and the reason to stderr (principle 12).
#[tokio::test]
async fn stay_awake_on_a_machine_it_cannot_hold_says_why_on_stderr() {
    let h = Harness::start_with(domux_server::testing::HarnessOptions {
        platform: Some("freebsd"),
        ..domux_server::testing::HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;
    let out = domux2(&h)
        .args(["stay-awake", "on"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Stay awake is off.\n");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("freebsd"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn stay_awake_status_with_no_server_says_so_rather_than_starting_one() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", dir.path().join("nothing.sock"))
        .env_remove("TMUX")
        .args(["stay-awake", "status"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("unknown"), "{said}");
    assert!(
        said.contains("server start"),
        "it names the way to know: {said}"
    );
}

/// The install writes two files that need a password, so without `--apply` it prints them and
/// writes nothing, the way the agent hook installers do. Full mode's files are macOS only
/// (decision 0029), so this runs only there.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn stay_awake_install_previews_both_files_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", dir.path().join("nothing.sock"))
        .env_remove("TMUX")
        .args(["stay-awake", "install", "--full"])
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("/Library/LaunchDaemons/"), "{said}");
    assert!(said.contains("/etc/sudoers.d/"), "{said}");
    assert!(said.contains("Nothing has been written."), "{said}");
    assert!(!Path::new(&domux_server::stay_awake::plist_path()).exists());
}

/// Elsewhere, full mode is one more flag on the same child (decision 0029): there are no files
/// to preview, so `install --full` says why on stderr rather than writing anything.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn stay_awake_install_full_needs_no_files_off_macos() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", dir.path().join("nothing.sock"))
        .env_remove("TMUX")
        .args(["stay-awake", "install", "--full"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Full mode needs no files on this system"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `install` with no mode named does nothing rather than guessing which one was meant.
#[tokio::test]
async fn stay_awake_install_with_no_mode_names_the_one_that_needs_files() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_domux2"))
        .env("DOMUX_SOCKET", dir.path().join("nothing.sock"))
        .env_remove("TMUX")
        .args(["stay-awake", "install"])
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("install --full"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty());
}
