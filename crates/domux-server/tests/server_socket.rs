//! The socket end to end: a client attaches and gets frames, a stale client is refused, and
//! the control API answers calls and streams events on the same socket.

use domux_core::api::{Request, Response};
use domux_core::proto::{
    encode, Capabilities, ClientMsg, Decoder, Hello, ServerMsg, PROTOCOL_VERSION,
};
use domux_server::pane::FakeSpawner;
use domux_server::process::FakeInspector;
use domux_server::{load_config, CoreDeps, FixedClock, Server, ServerOptions};
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
        deps: CoreDeps {
            spawner: Arc::new(FakeSpawner::default()),
            inspector: Arc::new(FakeInspector::default()),
            clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
            id_seed: 7,
        },
    };
    (Server::start(opts).await.unwrap(), dir)
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
