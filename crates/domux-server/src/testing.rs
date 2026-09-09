//! The headless harness: an in-process server on a temp socket plus clients that speak the
//! attach protocol and keep a buffer. Frames render in the golden format of
//! `domux_term::Grid::to_text` so a failing test prints a picture.

use crate::core::CoreMsg;
use crate::pane::{FakeSpawner, PtySpawner, RealSpawner};
use crate::process::{FakeInspector, ForegroundProcess, ProcessInspector};
use crate::{load_config, CoreDeps, FixedClock, LoadedConfig, Server, ServerHandle, ServerOptions};
use domux_core::api::{ApiError, Request, Response};
use domux_core::config::Config;
use domux_core::facts::{Fact, FactKey};
use domux_core::ids::{ClientId, PaneId, TabId, WorkspaceId};
use domux_core::keymap::{KeyName, Keymap};
use domux_core::model::Model;
use domux_core::proto::{
    encode, Capabilities, ClientMsg, CursorState, Decoder, FrameDiff, Hello, ServerMsg, WireColor,
    PROTOCOL_VERSION,
};
use domux_term::{
    Attrs, Cell, Color, Cursor, CursorShape, Grid, Key, KeyAction, KeyEvent, Mods, Rgb, Size,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color as RColor, Modifier};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// How long the harness waits for the server to reach a state it asked for. Long enough
/// that a loaded machine does not fail the test, short enough that a hang is a failure with
/// a message rather than a stuck CI job.
const SETTLE: Duration = Duration::from_secs(5);

/// Runs `f` on its own thread and fails, rather than hanging, when it does not finish inside
/// `limit`. `what` names the call in the failure.
///
/// For the synchronous tests whose regression is a hang and not a wrong answer: a test binary
/// has no way to report "this never finished", so a lost bound would stall the whole suite
/// silently instead of failing one test. The same reasoning as the outer `tokio::time::timeout`
/// around `wait_for_fact` in `tests/facts_harness.rs`, for code that is not async.
///
/// A thread that outlives its limit is left running: joining it is the hang this avoids. The
/// test harness ends the process when the run is over.
pub fn finishes_within<T: Send + 'static>(
    limit: Duration,
    what: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(limit) {
        Ok(value) => value,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("{what} did not finish within {limit:?}")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("{what} panicked"),
    }
}

pub struct HarnessOptions {
    pub config: Config,
    pub cols: u16,
    pub rows: u16,
    /// Real shells through portable-pty. Default false: the fake spawner, no processes.
    pub real_ptys: bool,
    /// Reuse a state directory, for resume tests. Default: a fresh temp directory.
    pub state_dir: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
    /// What the fake inspector reports as every pane's foreground command. Default `sh`.
    pub foreground: Option<String>,
    /// Who observes the facts. Default empty, matching `ServerOptions.providers`: a test
    /// asks for a provider by name here rather than shelling out to git or `gh` by default.
    pub providers: Vec<Arc<dyn crate::facts::FactProvider>>,
}

impl HarnessOptions {
    pub fn new(config: Config, cols: u16, rows: u16) -> HarnessOptions {
        HarnessOptions {
            config,
            cols,
            rows,
            real_ptys: false,
            state_dir: None,
            project_root: None,
            foreground: None,
            providers: Vec::new(),
        }
    }
}

struct HeadlessClient {
    writer: tokio::net::unix::OwnedWriteHalf,
    rx: mpsc::Receiver<ServerMsg>,
    buffer: Buffer,
    cursor: Option<CursorState>,
    clipboard: Vec<String>,
    detached: Option<String>,
    bells: usize,
}

pub struct Harness {
    /// The first attached client.
    pub client: ClientId,
    server: Option<ServerHandle>,
    clients: HashMap<ClientId, HeadlessClient>,
    pub spawner: Option<Arc<FakeSpawner>>,
    pub inspector: Arc<FakeInspector>,
    _tmp: tempfile::TempDir,
    /// Temp directories the harness made on a caller's behalf, kept alive until it drops:
    /// `git_project` hands back a path inside one, and a caller that had to bind the temp
    /// directory itself would be one `let _` away from a repository deleted mid-test.
    kept: Vec<tempfile::TempDir>,
    state_dir: PathBuf,
    project_root: PathBuf,
    socket: PathBuf,
    config: Config,
    cols: u16,
    rows: u16,
    providers: Vec<Arc<dyn crate::facts::FactProvider>>,
}

impl Harness {
    pub async fn start(config: Config, cols: u16, rows: u16) -> Harness {
        Harness::start_with(HarnessOptions::new(config, cols, rows)).await
    }

    pub async fn start_with(opts: HarnessOptions) -> Harness {
        let tmp = tempfile::tempdir().expect("temp dir");
        let state_dir = opts
            .state_dir
            .clone()
            .unwrap_or_else(|| tmp.path().join("state"));
        let project_root = opts
            .project_root
            .clone()
            .unwrap_or_else(|| tmp.path().join("proj"));
        std::fs::create_dir_all(&project_root).unwrap();
        let socket = tmp.path().join(domux_core::names::SOCKET_FILE_NAME);
        let inspector = Arc::new(FakeInspector::default());
        inspector.set(
            Some(ForegroundProcess {
                pid: 1,
                name: opts.foreground.clone().unwrap_or_else(|| "sh".into()),
            }),
            None,
        );
        let mut config = opts.config.clone();
        if opts.real_ptys && config.terminal.shell.is_none() {
            config.terminal.shell = Some("/bin/sh".into());
        }
        let spawner: Option<Arc<FakeSpawner>> = if opts.real_ptys {
            None
        } else {
            Some(Arc::new(FakeSpawner::default()))
        };
        let mut h = Harness {
            client: ClientId("c_0000".into()),
            server: None,
            clients: HashMap::new(),
            spawner,
            inspector,
            _tmp: tmp,
            kept: Vec::new(),
            state_dir,
            project_root,
            socket,
            config,
            cols: opts.cols,
            rows: opts.rows,
            providers: opts.providers,
        };
        h.start_server().await;
        h.client = h.attach(opts.cols, opts.rows).await;
        h
    }

    async fn start_server(&mut self) {
        let spawner: Arc<dyn PtySpawner> = match &self.spawner {
            Some(fake) => fake.clone(),
            None => Arc::new(RealSpawner),
        };
        let inspector: Arc<dyn ProcessInspector> = self.inspector.clone();
        let config_path = self.state_dir.join("domux.toml");
        let mut loaded: LoadedConfig = load_config(&config_path);
        if loaded.error.is_none() && !config_path.exists() {
            loaded.config = self.config.clone();
            loaded.keymap = Keymap::from_config(&self.config.keys)
                .expect("test config keymap")
                .0;
        }
        let opts = ServerOptions {
            socket_path: self.socket.clone(),
            state_dir: self.state_dir.clone(),
            config: loaded,
            project_root: self.project_root.clone(),
            providers: self.providers.clone(),
            deps: CoreDeps {
                spawner,
                inspector,
                clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
                id_seed: 7,
            },
        };
        self.server = Some(Server::start(opts).await.expect("server starts"));
    }

    /// Writes a config file into the harness state dir and reloads. For config tests.
    pub fn config_path(&self) -> PathBuf {
        self.state_dir.join("domux.toml")
    }

    pub async fn attach(&mut self, cols: u16, rows: u16) -> ClientId {
        let stream = UnixStream::connect(&self.socket).await.expect("connect");
        let (mut reader, mut writer) = stream.into_split();
        let hello = ClientMsg::Hello(Hello {
            version: domux_core::VERSION.into(),
            protocol: PROTOCOL_VERSION,
            cols,
            rows,
            caps: Capabilities {
                truecolor: true,
                osc52: true,
                ..Default::default()
            },
        });
        writer.write_all(&encode(&hello).unwrap()).await.unwrap();
        let (tx, rx) = mpsc::channel(1024);
        tokio::spawn(async move {
            let mut dec = Decoder::default();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                while let Ok(Some(msg)) = dec.next::<ServerMsg>() {
                    if tx.send(msg).await.is_err() {
                        return;
                    }
                }
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => dec.push(&buf[..n]),
                }
            }
        });
        let mut client = HeadlessClient {
            writer,
            rx,
            buffer: Buffer::empty(Rect::new(0, 0, cols, rows)),
            cursor: None,
            clipboard: Vec::new(),
            detached: None,
            bells: 0,
        };
        let id = match tokio::time::timeout(SETTLE, client.rx.recv())
            .await
            .expect("welcome in time")
            .expect("open")
        {
            ServerMsg::Welcome { client, .. } => client,
            ServerMsg::Refused { reason } => panic!("refused: {reason}"),
            other => panic!("unexpected first message {other:?}"),
        };
        self.clients.insert(id.clone(), client);
        // The welcome is sent while the core is still handling the attach, so it can reach
        // here before the core finishes the batch and publishes the model. Every caller
        // that asks the harness about this client - `focused_pane`, `current_tab` - reads
        // that snapshot, so hand back a client the model already holds rather than one that
        // is a scheduling accident away from being there.
        self.settle_until(
            |m| m.client(&id).is_some(),
            "the model never held the new client",
        )
        .await;
        id
    }

    /// Waits until the published model satisfies `ready`, or panics after `SETTLE`.
    async fn settle_until(&self, ready: impl Fn(&Model) -> bool, what: &str) {
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            if ready(&self.model()) {
                return;
            }
            assert!(tokio::time::Instant::now() < deadline, "{what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn send(&mut self, client: &ClientId, msg: ClientMsg) {
        let c = self.clients.get_mut(client).expect("known client");
        c.writer
            .write_all(&encode(&msg).unwrap())
            .await
            .expect("send");
    }

    /// A key by its config name: `C-a`, `s`, `Enter`, `Esc`, `S-Left`.
    pub async fn key(&mut self, client: ClientId, key: &str) {
        let name = KeyName::parse(key).unwrap_or_else(|e| panic!("{key}: {e}"));
        let mut mods = name.mods;
        if let Key::Char(c) = name.key {
            if c.is_uppercase() {
                mods |= Mods::SHIFT;
            }
        }
        self.send(
            &client,
            ClientMsg::Key(KeyEvent {
                key: name.key,
                mods,
                action: KeyAction::Press,
            }),
        )
        .await;
    }

    pub async fn type_text(&mut self, client: ClientId, text: &str) {
        for ch in text.chars() {
            let mods = if ch.is_uppercase() {
                Mods::SHIFT
            } else {
                Mods::empty()
            };
            self.send(
                &client,
                ClientMsg::Key(KeyEvent {
                    key: Key::Char(ch),
                    mods,
                    action: KeyAction::Press,
                }),
            )
            .await;
        }
    }

    pub async fn paste(&mut self, client: ClientId, text: &str) {
        self.send(&client, ClientMsg::Paste(text.to_string())).await;
    }

    pub async fn scroll(&mut self, client: ClientId, column: u16, row: u16, lines: i16) {
        self.send(&client, ClientMsg::Scroll { column, row, lines })
            .await;
    }

    pub async fn resize(&mut self, client: ClientId, cols: u16, rows: u16) {
        self.clients.get_mut(&client).unwrap().buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
        self.send(&client, ClientMsg::Resize { cols, rows }).await;
    }

    pub async fn detach(&mut self, client: ClientId) {
        self.send(&client, ClientMsg::Detach).await;
    }

    /// One control API call over a fresh connection.
    pub async fn api(&mut self, method: &str, params: Value) -> Result<Value, ApiError> {
        let s = UnixStream::connect(&self.socket).await.expect("connect");
        let (r, mut w) = s.into_split();
        let req = Request {
            id: Value::from(1),
            method: method.into(),
            params,
        };
        w.write_all(format!("{}\n", serde_json::to_string(&req).unwrap()).as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        // Bounded: a call the server never answers must fail with the method's name, not
        // hang until CI's own timeout kills the job and says nothing.
        tokio::time::timeout(SETTLE, BufReader::new(r).read_line(&mut line))
            .await
            .unwrap_or_else(|_| panic!("{method} was not answered within {SETTLE:?}"))
            .unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        match (resp.result, resp.error) {
            (Some(v), None) => Ok(v),
            (None, Some(e)) => Err(e),
            other => panic!("malformed response {other:?}"),
        }
    }

    /// Applies every message the server has sent so far, waiting 50 ms for more after the
    /// last one, then renders the client's buffer as text.
    pub async fn frame(&mut self, client: ClientId) -> String {
        self.pump(&client, Duration::from_millis(50)).await;
        self.render(&client)
    }

    async fn pump(&mut self, client: &ClientId, quiet: Duration) {
        let c = self.clients.get_mut(client).expect("known client");
        // Ends on the quiet window elapsing (`Err`) or the stream closing (`Ok(None)`).
        while let Ok(Some(msg)) = tokio::time::timeout(quiet, c.rx.recv()).await {
            apply(c, msg);
        }
    }

    fn render(&self, client: &ClientId) -> String {
        let c = self.clients.get(client).expect("known client");
        let mut grid = Grid::new(Size {
            cols: c.buffer.area.width,
            rows: c.buffer.area.height,
        });
        for (i, cell) in c.buffer.content.iter().enumerate() {
            let (x, y) = c.buffer.pos_of(i);
            let out: &mut Cell = grid.cell_mut(y, x);
            out.text = cell.symbol().into();
            out.width = domux_core::text::display_width(cell.symbol()).clamp(0, 2) as u8;
            if cell.symbol().is_empty() {
                out.width = 0;
            }
            out.fg = from_rcolor(cell.fg);
            out.bg = from_rcolor(cell.bg);
            out.underline_color = if cell.underline_color == RColor::Reset {
                None
            } else {
                Some(from_rcolor(cell.underline_color))
            };
            out.attrs = attrs_from(cell.modifier);
        }
        // The cell after a wide glyph is its spacer: width 0 so `to_text` prints it once.
        for y in 0..c.buffer.area.height {
            for x in 0..c.buffer.area.width.saturating_sub(1) {
                if grid.cell(y, x).width == 2 {
                    let spacer = grid.cell_mut(y, x + 1);
                    spacer.width = 0;
                    spacer.text.clear();
                }
            }
        }
        let cursor = match &c.cursor {
            Some(cs) => Cursor {
                row: cs.y,
                col: cs.x,
                visible: true,
                shape: cs.shape,
                blink: cs.blink,
            },
            None => Cursor {
                visible: false,
                ..Cursor::default()
            },
        };
        grid.to_text(&cursor)
    }

    /// Polls `frame` until `pred` holds or `timeout` passes; panics with the last frame.
    pub async fn wait_for(
        &mut self,
        client: ClientId,
        pred: impl Fn(&str) -> bool,
        timeout: Duration,
    ) -> String {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let f = self.frame(client.clone()).await;
            if pred(&f) {
                return f;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("condition not met within {timeout:?}; last frame:\n{f}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The model as of the last batch the core finished. Call `frame` first when the change
    /// you want to see was just requested.
    pub fn model(&self) -> Model {
        self.server
            .as_ref()
            .expect("server")
            .snapshot
            .lock()
            .unwrap()
            .clone()
    }

    /// The fact at `key` as of the last batch the core finished, published beside the model
    /// (see `ServerHandle::facts`). `None` when the fact is absent, whether because no
    /// provider has answered yet or because the last answer was absence.
    pub fn fact(&self, key: &FactKey) -> Option<Fact> {
        self.server
            .as_ref()
            .expect("server")
            .facts
            .lock()
            .unwrap()
            .get(key)
            .cloned()
    }

    /// Polls the published fact at `key` until `pred` holds, or panics after `timeout` with
    /// the last value seen. A fact takes at least one tick to arrive (Task 8's providers run
    /// off the core, on an interval), so a test that wants one waits for it here rather than
    /// sleeping a guessed-at duration and hoping the provider was faster.
    pub async fn wait_for_fact(
        &self,
        key: &FactKey,
        pred: impl Fn(Option<&Fact>) -> bool,
        timeout: Duration,
    ) -> Option<Fact> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let fact = self.fact(key);
            if pred(fact.as_ref()) {
                return fact;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("condition on {key} not met within {timeout:?}; last fact: {fact:?}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub fn focused_pane(&self, client: ClientId) -> PaneId {
        let m = self.model();
        m.client_tab(&client)
            .map(|t| t.focused.clone())
            .expect("client has a tab")
    }

    /// A pane's emulator size: the screen its program believes it has, which is what the
    /// smallest client on the tab gives it. Published beside the model after every batch,
    /// so call `frame` first when the resize you want to see was only just requested.
    pub fn pane_size(&self, pane: &PaneId) -> Size {
        self.server
            .as_ref()
            .expect("server")
            .pane_sizes
            .lock()
            .unwrap()
            .get(pane)
            .copied()
            .unwrap_or_else(|| panic!("no pane {pane}"))
    }

    /// Whether the server still holds a runtime for `pane`: its PTY and its emulator.
    /// Published beside the model, so call `frame` first when the change you want to see
    /// was only just requested. A pane whose record has gone but whose runtime has not is a
    /// process nothing will ever close.
    pub fn pane_is_running(&self, pane: &PaneId) -> bool {
        self.server
            .as_ref()
            .expect("server")
            .pane_sizes
            .lock()
            .unwrap()
            .contains_key(pane)
    }

    pub fn current_tab(&self, client: ClientId) -> TabId {
        self.model()
            .client(&client)
            .map(|c| c.tab.clone())
            .expect("client")
    }

    /// Types a command and Enter into a pane. Needs real PTYs.
    pub async fn spawn_in_pane(&mut self, pane: PaneId, command: &[&str]) {
        assert!(
            self.spawner.is_none(),
            "spawn_in_pane needs real PTYs: use Harness::start_with(HarnessOptions {{ real_ptys: true, .. }})"
        );
        let line = format!("{}\n", command.join(" "));
        self.api(
            "pane.send_text",
            serde_json::json!({"pane": pane.as_str(), "text": line}),
        )
        .await
        .expect("send_text");
    }

    /// Bytes as if the pane's program printed them. Works with fake and real PTYs.
    pub async fn feed_pane(&mut self, pane: PaneId, bytes: &[u8]) {
        self.core_tx()
            .send(CoreMsg::PaneOutput {
                pane,
                bytes: bytes.to_vec(),
            })
            .await
            .unwrap();
    }

    /// As if the pane's child exited.
    pub async fn exit_pane(&mut self, pane: PaneId, status: Option<i32>) {
        self.core_tx()
            .send(CoreMsg::PaneExited { pane, status })
            .await
            .unwrap();
    }

    /// Everything written to a fake pane's PTY: the bytes the program would have received.
    pub fn pane_input(&self, pane: &PaneId) -> Vec<u8> {
        self.spawner
            .as_ref()
            .expect("pane_input needs fake PTYs")
            .written(pane)
    }

    pub async fn clipboard(&mut self, client: ClientId) -> Vec<String> {
        self.pump(&client, Duration::from_millis(50)).await;
        self.clients[&client].clipboard.clone()
    }

    /// How many bells the server has rung at this client.
    pub async fn bells(&mut self, client: ClientId) -> usize {
        self.pump(&client, Duration::from_millis(50)).await;
        self.clients[&client].bells
    }

    pub async fn detached_reason(&mut self, client: ClientId) -> Option<String> {
        self.pump(&client, Duration::from_millis(50)).await;
        self.clients[&client].detached.clone()
    }

    pub fn set_foreground(&self, name: &str) {
        self.inspector.set(
            Some(ForegroundProcess {
                pid: 1,
                name: name.into(),
            }),
            None,
        );
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// The socket this harness server listens on, for a test that drives a real client or
    /// the CLI through `DOMUX_SOCKET`.
    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    fn core_tx(&self) -> mpsc::Sender<CoreMsg> {
        self.server.as_ref().expect("server").core_tx.clone()
    }

    /// Registers a temporary git repository as a project and returns its root.
    ///
    /// The repository has an origin, one commit and `origin/HEAD` set, so
    /// `git::default_branch` resolves rather than falling back. The temp directory it lives
    /// in is kept by the harness, so a caller does not have to bind one to keep the
    /// repository alive for the length of the test.
    ///
    /// `git_project_with_two_slots` builds slots on top of this one.
    pub async fn git_project(&mut self, default_branch: &str) -> PathBuf {
        let (tmp, repo) = repo_with_origin(default_branch);
        self.kept.push(tmp);
        self.api(
            "project.add",
            serde_json::json!({ "path": repo.to_str().expect("a temp path is utf-8") }),
        )
        .await
        .expect("project.add");
        repo
    }

    /// A git project with `workspace-1` and `workspace-2` made through `workspace.create`,
    /// so the worktrees on disk and the records in the model are the ones the server itself
    /// would have built. Returns the project root and the two workspace ids, in slot order.
    ///
    /// It runs two real creates, so it fetches and adds two worktrees: a test that only needs
    /// a project should call `git_project`.
    pub async fn git_project_with_two_slots(&mut self) -> (PathBuf, WorkspaceId, WorkspaceId) {
        let root = self.git_project("main").await;
        // By id, not by name: the harness always holds a second project of its own, and a
        // create that named neither would build its slot in whichever one the first client
        // happens to be looking at.
        let canonical = root.canonicalize().expect("the project root is there");
        let project = self
            .model()
            .project_at(&canonical)
            .map(|p| p.id.to_string())
            .expect("git_project registered the repository");
        let mut made = Vec::new();
        for _ in 0..2 {
            let created = self
                .api(
                    "workspace.create",
                    serde_json::json!({ "project": project }),
                )
                .await;
            made.push(created);
        }
        let ids: Vec<WorkspaceId> = made
            .into_iter()
            .map(|created| {
                let created = created.expect("workspace.create");
                WorkspaceId(
                    created["id"]
                        .as_str()
                        .expect("a create answers with an id")
                        .to_string(),
                )
            })
            .collect();
        (root, ids[0].clone(), ids[1].clone())
    }

    /// The first pane of a workspace's first tab, for a test that reads what was typed into a
    /// workspace that is not the client's.
    ///
    /// Waits for the tab: a create answers its caller from inside the batch that made the
    /// workspace, so the snapshot this reads can be one batch behind the answer.
    pub async fn first_pane_of(&mut self, workspace: &str) -> PaneId {
        let id = WorkspaceId(workspace.to_string());
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let found = self
                .model()
                .workspace(&id)
                .and_then(|w| w.tabs.first().map(|t| t.focused.clone()));
            if let Some(pane) = found {
                return pane;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "workspace {workspace} had no tab within {SETTLE:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Stops the server (persisting) and drops every client. The state dir stays.
    pub async fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.stop().await;
        }
        self.clients.clear();
    }

    /// Starts a new server on the same state dir and attaches a fresh first client. The
    /// fake spawner is kept, so its recorded requests accumulate across restarts.
    pub async fn restart(&mut self) {
        self.stop().await;
        self.start_server().await;
        self.client = self.attach(self.cols, self.rows).await;
    }
}

fn apply(c: &mut HeadlessClient, msg: ServerMsg) {
    match msg {
        ServerMsg::Frame(FrameDiff {
            full,
            cols,
            rows,
            cells,
            cursor,
        }) => {
            if full || c.buffer.area.width != cols || c.buffer.area.height != rows {
                c.buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
            }
            for u in cells {
                if u.x >= cols || u.y >= rows {
                    continue;
                }
                let cell = &mut c.buffer[(u.x, u.y)];
                cell.set_symbol(&u.symbol);
                cell.fg = to_rcolor(&u.fg);
                cell.bg = to_rcolor(&u.bg);
                cell.underline_color = u.underline.as_ref().map(to_rcolor).unwrap_or(RColor::Reset);
                cell.modifier = Modifier::from_bits_truncate(u.modifiers);
            }
            c.cursor = cursor;
        }
        ServerMsg::Clipboard(text) => c.clipboard.push(text),
        ServerMsg::Bell => c.bells += 1,
        ServerMsg::Detached { reason } => c.detached = Some(reason),
        ServerMsg::Welcome { .. } | ServerMsg::Refused { .. } => {}
    }
}

fn to_rcolor(c: &WireColor) -> RColor {
    match c {
        WireColor::Reset => RColor::Reset,
        WireColor::Indexed(i) => RColor::Indexed(*i),
        WireColor::Rgb(r, g, b) => RColor::Rgb(*r, *g, *b),
    }
}

fn from_rcolor(c: RColor) -> Color {
    match c {
        RColor::Reset => Color::Default,
        RColor::Indexed(i) => Color::Indexed(i),
        RColor::Rgb(r, g, b) => Color::Rgb(Rgb { r, g, b }),
        other => Color::Indexed(crate::client::wire_index(other)),
    }
}

fn attrs_from(m: Modifier) -> Attrs {
    let mut a = Attrs::empty();
    if m.contains(Modifier::BOLD) {
        a |= Attrs::BOLD;
    }
    if m.contains(Modifier::DIM) {
        a |= Attrs::DIM;
    }
    if m.contains(Modifier::ITALIC) {
        a |= Attrs::ITALIC;
    }
    if m.contains(Modifier::UNDERLINED) {
        a |= Attrs::UNDERLINE;
    }
    if m.contains(Modifier::SLOW_BLINK) {
        a |= Attrs::BLINK;
    }
    if m.contains(Modifier::REVERSED) {
        a |= Attrs::INVERSE;
    }
    if m.contains(Modifier::HIDDEN) {
        a |= Attrs::INVISIBLE;
    }
    if m.contains(Modifier::CROSSED_OUT) {
        a |= Attrs::STRIKETHROUGH;
    }
    a
}

/// The nth `|...|` row of a frame, without the trailing newline.
pub fn row(frame: &str, n: usize) -> &str {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .nth(n)
        .unwrap_or_else(|| panic!("frame has no row {n}:\n{frame}"))
}

pub fn shape_name(shape: CursorShape) -> &'static str {
    match shape {
        CursorShape::Block => "block",
        CursorShape::Underline => "underline",
        CursorShape::Bar => "bar",
    }
}

/// Temporary git repositories for tests. Here rather than in `tests/support` so the harness
/// and the tests that drive it build repositories the same way; `tests/support/mod.rs`
/// re-exports these.
/// Runs git and returns its trimmed stdout, panicking with stderr on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A commit with one file, so a repository has history to branch from.
pub fn commit(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-q", "-m", &format!("Add {name}")]);
}

/// A bare origin and a clone of it with one commit on `branch` and `origin/HEAD` set, which
/// is what `git::default_branch` reads. Returns the temp dir (keep it alive) and the clone.
pub fn repo_with_origin(branch: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    let work = tmp.path().join("audrey-app");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // Every other git call in these tests names its directory with `-C`. This one takes the
    // repository as an argument instead, so it is given an explicit working directory as well:
    // without one it would run in the test binary's own directory, inside a real checkout. A
    // bare init that failed would surface later as a confusing push error, so read its status
    // rather than dropping it.
    let status = Command::new("git")
        .current_dir(tmp.path())
        .args(["init", "-q", "--bare", "-b", branch])
        .arg(&origin)
        .status()
        .unwrap();
    assert!(status.success(), "git init --bare in {}", origin.display());
    git(&work, &["init", "-q", "-b", branch]);
    git(&work, &["config", "user.email", "test@example.com"]);
    git(&work, &["config", "user.name", "domux test"]);
    // The author's own git configuration reaches these repositories otherwise, and a global
    // `commit.gpgsign` would have these tests try to sign, a global `core.hooksPath` would run
    // that machine's hooks inside them. Repository configuration wins over global for every
    // command against this repository, including the ones that go through `git::run` and the
    // ones that run in its worktrees, so the isolation belongs here and not in production code.
    let no_hooks = tmp.path().join("no-hooks");
    git(
        &work,
        &["config", "core.hooksPath", no_hooks.to_str().unwrap()],
    );
    git(&work, &["config", "commit.gpgsign", "false"]);
    // The push below runs origin's receive hooks, so origin needs the same.
    git(
        &origin,
        &["config", "core.hooksPath", no_hooks.to_str().unwrap()],
    );
    commit(&work, "README.md", "hello\n");
    git(
        &work,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&work, &["push", "-q", "-u", "origin", branch]);
    git(&work, &["remote", "set-head", "origin", branch]);
    (tmp, work)
}
