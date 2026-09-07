//! End to end tests for the `domux2` binary: every subcommand is run as the process a
//! person types, against a harness server on a temp socket.
//!
//! The commands run through `tokio::process` rather than `std::process`, because the
//! harness server lives on this test's own runtime: a blocking `output()` would stop the
//! server it is waiting for.

use domux_core::config::Config;
use domux_server::testing::Harness;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
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
        Duration::from_secs(2),
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
            Duration::from_secs(2),
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
        Duration::from_secs(2),
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
    h.feed_pane(pane.clone(), b"alpha\r\nbeta").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("beta"),
        Duration::from_secs(2),
    )
    .await;
    let out = domux2(&h)
        .env("DOMUX_PANE", pane.as_str())
        .args(["pane", "read", "--lines", "2"])
        .output()
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "alpha\nbeta\n");
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
        Duration::from_secs(2),
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

/// Principle 13: the client's own terminal handling is only proved on a real terminal.
#[tokio::test]
async fn attach_inside_a_pty_draws_the_screen_and_leader_d_detaches_cleanly() {
    let h = Harness::start(Config::default(), 40, 10).await;
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 12,
            cols: 50,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_domux2"));
    cmd.arg("attach");
    cmd.env("DOMUX_SOCKET", h.socket_path());
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("TMUX");
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    // The pty only reads on a blocking thread; the test itself never blocks, so the
    // harness server on this runtime keeps drawing.
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
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while let Ok(chunk) = rx.try_recv() {
            output.extend(chunk);
        }
        if visible(&output).contains("┌ sh") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no screen within 10 s:\n{}",
            visible(&output)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    writer.write_all(b"\x01d").unwrap(); // C-a then d
    writer.flush().unwrap();
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
    assert!(log.contains("Not a directory"), "{log}");
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
