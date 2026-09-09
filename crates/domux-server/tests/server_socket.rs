//! The socket end to end: a client attaches and gets frames, a stale client is refused, and
//! the control API answers calls and streams events on the same socket.

use anyhow::Result;
use domux_core::api::{Request, Response};
use domux_core::proto::{
    encode, Capabilities, ClientMsg, Decoder, Hello, ServerMsg, PROTOCOL_VERSION,
};
use domux_server::core::CoreMsg;
use domux_server::pane::{FakeSpawner, PtyHandle, PtySpawner, SpawnRequest};
use domux_server::process::FakeInspector;
use domux_server::{load_config, CoreDeps, FixedClock, Server, ServerOptions};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Every wait in this file is bounded. A socket test that hangs blocks CI for its whole
/// timeout and says nothing; one that fails names what never arrived.
const DEADLINE: Duration = Duration::from_secs(10);

async fn start() -> (domux_server::ServerHandle, tempfile::TempDir) {
    start_with_config("none.toml").await
}

/// A server in its own temporary directory, reading `config` from that directory. The name
/// need not exist: a missing config file is the ordinary case.
async fn start_with_config(config: &str) -> (domux_server::ServerHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let opts = ServerOptions {
        socket_path: dir.path().join("s.sock"),
        state_dir: dir.path().join("state"),
        config: load_config(&dir.path().join(config)),
        project_root: project,
        providers: Vec::new(),
        deps: CoreDeps {
            spawner: Arc::new(FakeSpawner::default()),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    (Server::start(opts).await.unwrap(), dir)
}

/// Every pane it starts answers with an immediate exit, which is what a `terminal.shell`
/// pointing at a binary that exits at once does.
#[derive(Default)]
struct ImmediateExitSpawner {
    inner: FakeSpawner,
    count: AtomicUsize,
}

impl PtySpawner for ImmediateExitSpawner {
    fn spawn(
        &self,
        req: SpawnRequest,
        tx: tokio::sync::mpsc::Sender<CoreMsg>,
    ) -> Result<Box<dyn PtyHandle>> {
        let pane = req.pane.clone();
        let pty = self.inner.spawn(req, tx.clone())?;
        self.count.fetch_add(1, Ordering::SeqCst);
        let _ = tx.try_send(CoreMsg::PaneExited {
            pane,
            status: Some(1),
        });
        Ok(pty)
    }
}

/// Exits every pane through `exit_through` starts and lets later ones live, so a workspace
/// can recover between two runs of the guard.
struct SwitchingExitSpawner {
    inner: FakeSpawner,
    count: AtomicUsize,
    exit_through: AtomicUsize,
}

impl PtySpawner for SwitchingExitSpawner {
    fn spawn(
        &self,
        req: SpawnRequest,
        tx: tokio::sync::mpsc::Sender<CoreMsg>,
    ) -> Result<Box<dyn PtyHandle>> {
        let pane = req.pane.clone();
        let pty = self.inner.spawn(req, tx.clone())?;
        let count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        if count <= self.exit_through.load(Ordering::SeqCst) {
            let _ = tx.try_send(CoreMsg::PaneExited {
                pane,
                status: Some(1),
            });
        }
        Ok(pty)
    }
}

fn hello(version: &str) -> ClientMsg {
    ClientMsg::Hello(Hello {
        version: version.into(),
        protocol: PROTOCOL_VERSION,
        cols: 40,
        rows: 10,
        caps: Capabilities::default(),
    })
}

async fn read_msg(stream: &mut UnixStream, dec: &mut Decoder) -> ServerMsg {
    let deadline = tokio::time::Instant::now() + DEADLINE;
    loop {
        if let Some(m) = dec.next::<ServerMsg>().unwrap() {
            return m;
        }
        let mut buf = [0u8; 4096];
        let n = tokio::time::timeout_at(deadline, stream.read(&mut buf))
            .await
            .expect("the server sent no message before the deadline")
            .unwrap();
        assert!(n > 0, "connection closed");
        dec.push(&buf[..n]);
    }
}

/// The next line of a control API stream, or a named failure rather than a hang.
async fn next_line<R: tokio::io::AsyncBufRead + Unpin>(lines: &mut tokio::io::Lines<R>) -> String {
    tokio::time::timeout(DEADLINE, lines.next_line())
        .await
        .expect("the server sent no line before the deadline")
        .unwrap()
        .expect("the server closed the stream")
}

#[tokio::test]
async fn attach_hello_gets_welcome_then_a_full_frame() {
    let (server, _dir) = start().await;
    let mut s = UnixStream::connect(&server.socket_path).await.unwrap();
    s.write_all(&encode(&hello(domux_core::VERSION)).unwrap())
        .await
        .unwrap();
    let mut dec = Decoder::default();
    match read_msg(&mut s, &mut dec).await {
        ServerMsg::Welcome { client, version } => {
            assert!(client.as_str().starts_with("c_"));
            assert_eq!(version, domux_core::VERSION);
        }
        other => panic!("{other:?}"),
    }
    match read_msg(&mut s, &mut dec).await {
        ServerMsg::Frame(f) => {
            assert!(f.full);
            assert_eq!((f.cols, f.rows), (40, 10));
            assert!(
                f.cells.iter().any(|c| c.symbol == "┌"),
                "a pane box was drawn"
            );
        }
        other => panic!("{other:?}"),
    }
    server.stop().await;
}

#[tokio::test]
async fn version_mismatch_is_refused_with_the_restart_instruction() {
    let (server, _dir) = start().await;
    let mut s = UnixStream::connect(&server.socket_path).await.unwrap();
    s.write_all(&encode(&hello("1.0.0")).unwrap())
        .await
        .unwrap();
    let mut dec = Decoder::default();
    match read_msg(&mut s, &mut dec).await {
        ServerMsg::Refused { reason } => assert_eq!(
            reason,
            format!(
                "the server is domux {} and this client is 1.0.0; run domux2 server restart",
                domux_core::VERSION
            )
        ),
        other => panic!("{other:?}"),
    }
    server.stop().await;
}

async fn call(path: &std::path::Path, method: &str, params: serde_json::Value) -> Response {
    let s = UnixStream::connect(path).await.unwrap();
    let (r, mut w) = s.into_split();
    let req = Request {
        id: serde_json::json!(1),
        method: method.into(),
        params,
    };
    w.write_all(format!("{}\n", serde_json::to_string(&req).unwrap()).as_bytes())
        .await
        .unwrap();
    let mut lines = BufReader::new(r).lines();
    serde_json::from_str(&next_line(&mut lines).await).unwrap()
}

#[tokio::test]
async fn control_api_answers_server_info_and_unknown_methods_on_the_same_socket() {
    let (server, _dir) = start().await;
    let ok = call(&server.socket_path, "server.info", serde_json::json!({})).await;
    let info = ok.result.expect("result");
    assert_eq!(info["version"], domux_core::VERSION);
    assert_eq!(info["clients"].as_array().unwrap().len(), 0);
    assert!(info["config_error"].is_null());
    let err = call(&server.socket_path, "pane.explode", serde_json::json!({})).await;
    assert_eq!(
        err.error.unwrap().code,
        domux_core::api::ErrorCode::NotFound
    );
    server.stop().await;
}

#[tokio::test]
async fn events_subscribe_streams_client_attached() {
    let (server, _dir) = start().await;
    let s = UnixStream::connect(&server.socket_path).await.unwrap();
    let (r, mut w) = s.into_split();
    w.write_all(
        b"{\"id\":1,\"method\":\"events.subscribe\",\"params\":{\"filter\":[\"client.*\"]}}\n",
    )
    .await
    .unwrap();
    let mut lines = BufReader::new(r).lines();
    let ack: Response = serde_json::from_str(&next_line(&mut lines).await).unwrap();
    assert!(ack.result.is_some());
    let mut c = UnixStream::connect(&server.socket_path).await.unwrap();
    c.write_all(&encode(&hello(domux_core::VERSION)).unwrap())
        .await
        .unwrap();
    let event: serde_json::Value = serde_json::from_str(&next_line(&mut lines).await).unwrap();
    assert_eq!(event["event"], "client.attached");
    assert!(event["client"].as_str().unwrap().starts_with("c_"));
    server.stop().await;
}

/// Polls `server.info` until it reports `want` clients, or names what it saw instead. The
/// server registers and forgets a client through the core task, so the answer arrives some
/// messages later; polling a bounded number of times says so without a sleep long enough to
/// be a guess or short enough to be a race.
async fn wait_for_clients(path: &std::path::Path, want: usize) {
    let deadline = tokio::time::Instant::now() + DEADLINE;
    loop {
        let info = call(path, "server.info", serde_json::json!({}))
            .await
            .result
            .expect("result");
        let found = info["clients"].as_array().unwrap().len();
        if found == want {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server still reports {found} clients, wanted {want}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A client that goes away without detaching still has to be forgotten. Nothing tells the
/// core on its own: the connection task holds the only notice, and the writer task it
/// spawned would otherwise wait forever on a channel the core keeps open.
#[tokio::test]
async fn a_client_that_drops_its_connection_is_forgotten() {
    let (server, _dir) = start().await;
    let mut s = UnixStream::connect(&server.socket_path).await.unwrap();
    s.write_all(&encode(&hello(domux_core::VERSION)).unwrap())
        .await
        .unwrap();
    let mut dec = Decoder::default();
    assert!(matches!(
        read_msg(&mut s, &mut dec).await,
        ServerMsg::Welcome { .. }
    ));
    wait_for_clients(&server.socket_path, 1).await;
    drop(s);
    wait_for_clients(&server.socket_path, 0).await;
    server.stop().await;
}

/// `stop` returns once `state.json` is written, so this needs no wait at all: a file that is
/// not there when `stop` returns is the defect, not a slow disk.
#[tokio::test]
async fn stop_writes_the_state_file_and_removes_the_socket() {
    let (server, dir) = start().await;
    let socket = server.socket_path.clone();
    server.stop().await;
    assert!(!socket.exists(), "the socket file is removed");
    let text = std::fs::read_to_string(dir.path().join("state").join("state.json"))
        .expect("state.json is on disk when stop returns");
    let file = domux_core::state_file::parse(&text).unwrap();
    assert_eq!(file.schema_version, domux_core::state_file::SCHEMA_VERSION);
    assert_eq!(file.projects.len(), 1, "the implicit folder project");
    assert_eq!(
        file.projects[0].workspaces[0].tabs.len(),
        1,
        "a workspace never has no tab"
    );
}

/// A bad config file never blocks startup (architecture spec section 9). The server answers,
/// and `server.info` names the line so a person can fix it.
#[tokio::test]
async fn a_broken_config_is_reported_and_the_server_still_starts() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let config = dir.path().join("domux.toml");
    std::fs::write(&config, "[keys\nleader = \"C-a\"\n").unwrap();
    let loaded = load_config(&config);
    assert!(loaded.error.is_some(), "a syntax error is reported");
    let opts = ServerOptions {
        socket_path: dir.path().join("s.sock"),
        state_dir: dir.path().join("state"),
        config: loaded,
        project_root: project,
        providers: Vec::new(),
        deps: CoreDeps {
            spawner: Arc::new(FakeSpawner::default()),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    let server = Server::start(opts).await.unwrap();
    let info = call(&server.socket_path, "server.info", serde_json::json!({}))
        .await
        .result
        .expect("result");
    let reported = info["config_error"].as_str().expect("config_error");
    assert!(
        reported.starts_with("domux.toml line 1:"),
        "the notice names the line: {reported}"
    );
    assert_eq!(info["config_file"], config.to_string_lossy().as_ref());
    server.stop().await;
}

/// A frame the decoder refuses ends the connection, and the client behind it has to be
/// forgotten just the same. This is the path where nothing else tells the core: the read
/// loop leaves through an error rather than through end of file, so the notice has to be
/// tied to the connection ending and not to one way of ending it.
#[tokio::test]
async fn a_client_whose_frame_cannot_be_decoded_is_forgotten() {
    let (server, _dir) = start().await;
    let mut s = UnixStream::connect(&server.socket_path).await.unwrap();
    s.write_all(&encode(&hello(domux_core::VERSION)).unwrap())
        .await
        .unwrap();
    let mut dec = Decoder::default();
    assert!(matches!(
        read_msg(&mut s, &mut dec).await,
        ServerMsg::Welcome { .. }
    ));
    wait_for_clients(&server.socket_path, 1).await;
    // A well-formed 4-byte frame whose body is no `ClientMsg` variant.
    s.write_all(&[0, 0, 0, 4, 0xff, 0xff, 0xff, 0xff])
        .await
        .unwrap();
    wait_for_clients(&server.socket_path, 0).await;
    server.stop().await;
}

#[tokio::test]
async fn a_shell_that_exits_immediately_has_bounded_respawns_and_keeps_the_server_responsive() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let mut config = load_config(&dir.path().join("none.toml"));
    config.config.terminal.shell = Some("/broken/shell".into());
    let spawner = Arc::new(ImmediateExitSpawner::default());
    let opts = ServerOptions {
        socket_path: dir.path().join("s.sock"),
        state_dir: dir.path().join("state"),
        config,
        project_root: project,
        providers: Vec::new(),
        deps: CoreDeps {
            spawner: spawner.clone(),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    let server = Server::start(opts).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        spawner.count.load(Ordering::SeqCst) <= 4,
        "immediate exits spawned {} panes",
        spawner.count.load(Ordering::SeqCst)
    );
    // The core is not wedged in a respawn loop: it still answers a call.
    let info = tokio::time::timeout(
        DEADLINE,
        call(&server.socket_path, "server.info", serde_json::json!({})),
    )
    .await
    .expect("the server still answers after the respawn guard trips");
    assert!(info.result.is_some());
    // The pane that exited is kept rather than replaced, and one client attaching now is
    // told which shell failed and which key sets it.
    let mut attached = UnixStream::connect(&server.socket_path).await.unwrap();
    let mut wide_hello = hello(domux_core::VERSION);
    let ClientMsg::Hello(h) = &mut wide_hello else {
        unreachable!();
    };
    h.cols = 100;
    attached
        .write_all(&encode(&wide_hello).unwrap())
        .await
        .unwrap();
    let mut decoder = Decoder::default();
    assert!(matches!(
        read_msg(&mut attached, &mut decoder).await,
        ServerMsg::Welcome { .. }
    ));
    let frame = match read_msg(&mut attached, &mut decoder).await {
        ServerMsg::Frame(frame) => frame,
        other => panic!("{other:?}"),
    };
    let screen: String = frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert!(
        screen.contains("shell /broken/shell exited immediately"),
        "the hint names the configured shell"
    );
    assert!(screen.contains("terminal.shell"), "the hint names the key");
    assert_eq!(
        server.snapshot.lock().unwrap().all_pane_ids().len(),
        1,
        "the exited pane is kept, not replaced"
    );
    // And it is still kept once it has aged past the immediate window. The block is held on
    // the workspace, not inferred from the pane's age, so nothing reconsiders the retained
    // pane and starts a slower respawn loop of one process every two seconds.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(
        spawner.count.load(Ordering::SeqCst) <= 4,
        "the guard still holds after the retained pane aged out: {} spawns",
        spawner.count.load(Ordering::SeqCst)
    );
    assert_eq!(
        server.snapshot.lock().unwrap().all_pane_ids().len(),
        1,
        "the retained pane is not replaced once it is older than the immediate window"
    );
    server.stop().await;
}

/// The guard is a rate limit, not a latch: a workspace whose shell lives past the immediate
/// window gets its whole allowance back, so fixing an rc file does not need a restart.
#[tokio::test]
async fn a_workspace_whose_shell_survives_gets_its_full_respawn_allowance_back() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let spawner = Arc::new(SwitchingExitSpawner {
        inner: FakeSpawner::default(),
        count: AtomicUsize::new(0),
        exit_through: AtomicUsize::new(2),
    });
    let opts = ServerOptions {
        socket_path: dir.path().join("s.sock"),
        state_dir: dir.path().join("state"),
        config: load_config(&dir.path().join("none.toml")),
        project_root: project,
        providers: Vec::new(),
        deps: CoreDeps {
            spawner: spawner.clone(),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    let server = Server::start(opts).await.unwrap();
    // Two immediate exits, then a third pane that lives.
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while spawner.count.load(Ordering::SeqCst) < 3 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the third pane was not spawned"
        );
        tokio::task::yield_now().await;
    }
    // Past the immediate window, and past a tick as well. The counter is cleared by
    // `reset_respawn_guards_for_surviving_panes`, which runs from `Core::after_batch`, so
    // waiting for the pane to get old is not enough on its own: a batch has to run between
    // the pane passing `IMMEDIATE_EXIT` and the exit below arriving. The periodic tick is
    // one second, so sleeping a full tick past the two second window guarantees a batch
    // instead of relying on incidental traffic to land in the gap. Waiting only 2100ms left
    // a 100ms gap and failed on a loaded runner, which read as "the replacements stopped at
    // 5 of 7". The deadline below cannot recover from it: once the exit is handled with the
    // guard still set, the allowance is spent and no amount of waiting spawns the rest.
    tokio::time::sleep(Duration::from_millis(3200)).await;
    spawner.exit_through.store(usize::MAX, Ordering::SeqCst);
    let surviving_pane = spawner.inner.requests().last().unwrap().pane.clone();
    server
        .core_tx
        .send(CoreMsg::PaneExited {
            pane: surviving_pane,
            status: Some(1),
        })
        .await
        .unwrap();
    // The survivor's own exit is not immediate, so it is replaced without counting; its
    // three replacements then exit at once and the fourth is kept. 3 + 1 + 3 = 7.
    //
    // Poll to the expected count rather than sleeping a fixed span and asserting once: the
    // work is four spawns deep and a loaded runner does not finish it in any span short
    // enough to keep this test quick. A fixed 200 ms saw 5 of 7 on CI.
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while spawner.count.load(Ordering::SeqCst) < 7 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the replacements stopped at {} of 7",
            spawner.count.load(Ordering::SeqCst)
        );
        tokio::task::yield_now().await;
    }
    // Then hold still: the fourth replacement must be kept, not replaced again.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        spawner.count.load(Ordering::SeqCst),
        7,
        "the guard let a fifth replacement through"
    );
    server.stop().await;
}

/// A client already attached when the shell starts failing is told so where it is, rather
/// than having to detach and come back for the hint.
#[tokio::test]
async fn a_client_attached_when_the_guard_trips_is_told_which_shell_failed() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let mut config = load_config(&dir.path().join("none.toml"));
    config.config.terminal.shell = Some("/broken/shell".into());
    // The first pane lives, so a client can attach before anything has failed.
    let spawner = Arc::new(SwitchingExitSpawner {
        inner: FakeSpawner::default(),
        count: AtomicUsize::new(0),
        exit_through: AtomicUsize::new(0),
    });
    let opts = ServerOptions {
        socket_path: dir.path().join("s.sock"),
        state_dir: dir.path().join("state"),
        config,
        project_root: project,
        providers: Vec::new(),
        deps: CoreDeps {
            spawner: spawner.clone(),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    let server = Server::start(opts).await.unwrap();
    let mut attached = UnixStream::connect(&server.socket_path).await.unwrap();
    let mut wide_hello = hello(domux_core::VERSION);
    let ClientMsg::Hello(h) = &mut wide_hello else {
        unreachable!();
    };
    h.cols = 100;
    attached
        .write_all(&encode(&wide_hello).unwrap())
        .await
        .unwrap();
    let mut decoder = Decoder::default();
    assert!(matches!(
        read_msg(&mut attached, &mut decoder).await,
        ServerMsg::Welcome { .. }
    ));
    assert!(matches!(
        read_msg(&mut attached, &mut decoder).await,
        ServerMsg::Frame(_)
    ));
    // Now the shell starts failing, within the immediate window of this pane's start.
    spawner.exit_through.store(usize::MAX, Ordering::SeqCst);
    let pane = spawner.inner.requests().last().unwrap().pane.clone();
    server
        .core_tx
        .send(CoreMsg::PaneExited {
            pane,
            status: Some(1),
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + DEADLINE;
    let mut screen = String::new();
    while !screen.contains("shell /broken/shell exited immediately") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the attached client was never told, last screen: {screen:?}"
        );
        if let ServerMsg::Frame(frame) = read_msg(&mut attached, &mut decoder).await {
            let text: String = frame
                .cells
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect();
            if frame.full {
                screen = text;
            } else {
                screen.push_str(&text);
            }
        }
    }
    assert!(screen.contains("terminal.shell"), "the hint names the key");
    assert!(
        spawner.count.load(Ordering::SeqCst) <= 4,
        "the guard still bounds the replacements: {} spawns",
        spawner.count.load(Ordering::SeqCst)
    );
    server.stop().await;
}

/// The bound holds wherever a pane is replaced, so a key cannot walk around it. Enter on a
/// pane whose child exited closes it, and closing a workspace's last pane brings a
/// replacement shell: with the guard consulted only where the exit was counted, each Enter
/// started another shell and key repeat was a spawn storm. A blocked workspace answers Enter
/// with the notice instead, and `config.reload` is the way back.
#[tokio::test]
async fn enter_on_a_retained_pane_starts_no_shell_until_the_config_is_reloaded() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let config_path = dir.path().join("domux.toml");
    std::fs::write(&config_path, "[terminal]\nshell = \"/broken/shell\"\n").unwrap();
    // Everything exits at once, so the guard trips on the fourth start.
    let spawner = Arc::new(SwitchingExitSpawner {
        inner: FakeSpawner::default(),
        count: AtomicUsize::new(0),
        exit_through: AtomicUsize::new(usize::MAX),
    });
    let opts = ServerOptions {
        socket_path: dir.path().join("s.sock"),
        state_dir: dir.path().join("state"),
        config: load_config(&config_path),
        project_root: project,
        providers: Vec::new(),
        deps: CoreDeps {
            spawner: spawner.clone(),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    let server = Server::start(opts).await.unwrap();
    let mut attached = UnixStream::connect(&server.socket_path).await.unwrap();
    let mut wide_hello = hello(domux_core::VERSION);
    let ClientMsg::Hello(h) = &mut wide_hello else {
        unreachable!();
    };
    h.cols = 100;
    attached
        .write_all(&encode(&wide_hello).unwrap())
        .await
        .unwrap();
    let mut decoder = Decoder::default();
    assert!(matches!(
        read_msg(&mut attached, &mut decoder).await,
        ServerMsg::Welcome { .. }
    ));
    let deadline = tokio::time::Instant::now() + DEADLINE;
    let mut screen = String::new();
    while !screen.contains("shell /broken/shell exited immediately") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the guard never tripped, last screen: {screen:?}"
        );
        if let ServerMsg::Frame(frame) = read_msg(&mut attached, &mut decoder).await {
            let text: String = frame
                .cells
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect();
            if frame.full {
                screen = text;
            } else {
                screen.push_str(&text);
            }
        }
    }
    let tripped = spawner.count.load(Ordering::SeqCst);
    assert_eq!(tripped, 4, "four starts, then the guard keeps the pane");
    let retained = server.snapshot.lock().unwrap().all_pane_ids();
    assert_eq!(retained.len(), 1, "the fourth pane is kept");
    // Key repeat on the retained pane: ten Enters, each after the last has been answered,
    // which is what a held key looks like to the server.
    let enter = encode(&ClientMsg::Key(domux_term::KeyEvent::press(
        domux_term::Key::Enter,
        domux_term::Mods::empty(),
    )))
    .unwrap();
    for _ in 0..10 {
        attached.write_all(&enter).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(
        spawner.count.load(Ordering::SeqCst),
        tripped,
        "Enter on the retained pane started another shell"
    );
    assert_eq!(
        server.snapshot.lock().unwrap().all_pane_ids(),
        retained,
        "Enter closed the retained pane rather than answering with the notice"
    );
    // `pane.close` reaches a replacement the same way, and is bounded the same way. The
    // workspace keeps a tab - one without a tab is a state no frame can draw - and the pane
    // in it has no process, which `pane.list` reports as a screen of 0x0 rather than a size
    // nothing is drawing.
    let closed = call(&server.socket_path, "pane.close", serde_json::json!({})).await;
    assert!(closed.result.is_some(), "{closed:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        spawner.count.load(Ordering::SeqCst),
        tripped,
        "closing the pane started another shell"
    );
    let listed = call(&server.socket_path, "pane.list", serde_json::json!({})).await;
    let listed = listed.result.expect("result");
    let panes = listed.as_array().expect("a list of panes");
    assert_eq!(panes.len(), 1, "the workspace kept exactly one pane");
    assert_eq!(panes[0]["cols"], 0, "and it has no process: {panes:?}");
    // The shell is fixed and the config reloaded: the workspace gets its allowance back and
    // the next shell starts. This one lives, so nothing trips the guard again.
    spawner.exit_through.store(tripped, Ordering::SeqCst);
    std::fs::write(&config_path, "[terminal]\nshell = \"/bin/sh\"\n").unwrap();
    let reloaded = call(&server.socket_path, "config.reload", serde_json::json!({})).await;
    assert!(reloaded.result.is_some(), "{reloaded:?}");
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while spawner.count.load(Ordering::SeqCst) == tripped {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no shell started after the config was reloaded"
        );
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        spawner.count.load(Ordering::SeqCst),
        tripped + 1,
        "the reload started one shell, and that one lives"
    );
    // A client attaching now is told nothing: the notice named a state that is over.
    let mut after = UnixStream::connect(&server.socket_path).await.unwrap();
    after
        .write_all(&encode(&wide_hello).unwrap())
        .await
        .unwrap();
    let mut decoder = Decoder::default();
    assert!(matches!(
        read_msg(&mut after, &mut decoder).await,
        ServerMsg::Welcome { .. }
    ));
    let frame = match read_msg(&mut after, &mut decoder).await {
        ServerMsg::Frame(frame) => frame,
        other => panic!("{other:?}"),
    };
    let screen: String = frame
        .cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert!(
        !screen.contains("exited immediately"),
        "the notice outlived the state it named: {screen:?}"
    );
    server.stop().await;
}

/// Only the workspace's last pane counts towards the guard. Closing any other one leaves
/// the workspace a tab, so nothing replaces it and there is no loop to bound - and counting
/// it would spend the allowance on ordinary short-lived commands in a split.
#[tokio::test]
async fn a_pane_that_is_not_the_workspaces_last_does_not_spend_the_respawn_allowance() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let spawner = Arc::new(SwitchingExitSpawner {
        inner: FakeSpawner::default(),
        count: AtomicUsize::new(0),
        exit_through: AtomicUsize::new(0),
    });
    let opts = ServerOptions {
        socket_path: dir.path().join("s.sock"),
        state_dir: dir.path().join("state"),
        config: load_config(&dir.path().join("none.toml")),
        project_root: project,
        providers: Vec::new(),
        deps: CoreDeps {
            spawner: spawner.clone(),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    let server = Server::start(opts).await.unwrap();
    let first = spawner.inner.requests().last().unwrap().pane.clone();
    let split = call(
        &server.socket_path,
        "pane.split",
        serde_json::json!({"pane": first.to_string(), "dir": "right"}),
    )
    .await;
    assert!(split.error.is_none(), "{:?}", split.error);
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while spawner.count.load(Ordering::SeqCst) < 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the split pane was not spawned"
        );
        tokio::task::yield_now().await;
    }
    let second = spawner.inner.requests().last().unwrap().pane.clone();
    // From here every new pane exits at once, well inside both panes' immediate window.
    spawner.exit_through.store(usize::MAX, Ordering::SeqCst);
    // The first pane is not the workspace's last, so its immediate exit is not counted and
    // brings no replacement.
    server
        .core_tx
        .send(CoreMsg::PaneExited {
            pane: first,
            status: Some(1),
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        spawner.count.load(Ordering::SeqCst),
        2,
        "closing a pane that is not the last one replaces nothing"
    );
    // The second pane is the last one, so it starts the count with a full allowance: three
    // replacements, and the fourth immediate exit is kept. 2 + 3 = 5 starts in all.
    server
        .core_tx
        .send(CoreMsg::PaneExited {
            pane: second,
            status: Some(1),
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(spawner.count.load(Ordering::SeqCst), 5);
    server.stop().await;
}

#[tokio::test]
async fn server_stop_replies_before_the_server_stops() {
    let (server, _dir) = start().await;
    let response = call(&server.socket_path, "server.stop", serde_json::json!({})).await;
    assert_eq!(response.result.unwrap(), serde_json::json!({"ok": true}));
    server.stop().await;
}

/// A region that parses but has no behaviour yet answers `Unavailable`, which is a different
/// arm from the `NotFound` an unknown method name or an absent client gets. The client is
/// attached first so the call gets past `Ctx::view`, which is the `NotFound` path.
///
/// The agents overlay and not the switcher: M2 built the switcher and the sidebar's box, and
/// those two now answer `Refused` when the thing they name is not on the screen, which is the
/// third arm and not this one.
#[tokio::test]
async fn a_region_that_arrives_in_a_later_milestone_returns_unavailable() {
    let (server, _dir) = start().await;
    let mut s = UnixStream::connect(&server.socket_path).await.unwrap();
    s.write_all(&encode(&hello(domux_core::VERSION)).unwrap())
        .await
        .unwrap();
    let mut dec = Decoder::default();
    assert!(matches!(
        read_msg(&mut s, &mut dec).await,
        ServerMsg::Welcome { .. }
    ));
    let response = call(
        &server.socket_path,
        "focus.region",
        serde_json::json!({"region": "agents_overlay"}),
    )
    .await;
    assert_eq!(
        response.error.unwrap().code,
        domux_core::api::ErrorCode::Unavailable
    );
    server.stop().await;
}

/// The socket is what access control rests on: anyone who can connect to it can spawn
/// processes in this user's panes. `bind` takes its mode from the umask, which is 022 on a
/// stock shell, so the socket arrived as `srwxr-xr-x` and any user on the machine could drive
/// the server. The directory around it was standing in for this, and only by accident: it is
/// whatever `DOMUX_SOCKET` points at.
#[tokio::test]
async fn the_socket_is_private_to_the_user_who_started_the_server() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // A world-writable parent, so nothing but the socket's own mode can be protecting it.
    let open = dir.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    let path = open.join("domux2.sock");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let handle = domux_server::socket::listen(&path, tx).await.unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "socket mode is {mode:o}, not 0600");
    // A directory the server did not create keeps the permissions it had.
    let dir_mode = std::fs::metadata(&open).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        dir_mode, 0o777,
        "the server narrowed a directory it does not own"
    );
    handle.abort();
}

/// The directory the server does create is its own, and is private from the moment it exists.
#[tokio::test]
async fn a_socket_directory_the_server_creates_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("made-by-domux").join("domux2.sock");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let handle = domux_server::socket::listen(&path, tx).await.unwrap();
    let mode = std::fs::metadata(path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "directory mode is {mode:o}, not 0700");
    handle.abort();
}
