//! The core task: the one owner of the Model, every PaneRuntime and every ClientConn.

use crate::api::{self, Ctx};
use crate::client::{ClientConn, Hint, HintKind};
use crate::pane::{new_pane_emulator, PaneRuntime, SpawnRequest, PANE_TERM};
use crate::render::{self, RenderInput};
use crate::{CoreDeps, LoadedConfig, ServerOptions};
use domux_core::api::{ApiError, Event, Method, Request, Response};
use domux_core::ids::{ClientId, PaneId, TabId, WorkspaceId};
use domux_core::keymap::Action;
use domux_core::model::{ClientView, ConfirmKind, Focus, Model, Overlay, PaneFacts, RegionKind};
use domux_core::proto::{ClientMsg, Hello, ServerMsg};
use domux_core::state_file::{self, StateFile};
use domux_term::{Emulator, Rgb, Size};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

pub enum CoreMsg {
    PaneOutput {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    PaneExited {
        pane: PaneId,
        status: Option<i32>,
    },
    ClientConnected {
        hello: Hello,
        tx: mpsc::Sender<ServerMsg>,
        reply: oneshot::Sender<Result<ClientId, String>>,
    },
    ClientInput {
        client: ClientId,
        msg: ClientMsg,
    },
    ClientGone {
        client: ClientId,
    },
    Api {
        request: Request,
        reply: oneshot::Sender<Response>,
    },
    Subscribe {
        filter: Vec<String>,
        tx: mpsc::Sender<Event>,
    },
    /// Once a second: the process inspector, the clock, exited-pane cleanup.
    Tick,
    Snapshot {
        reply: oneshot::Sender<Model>,
    },
    Shutdown,
}

/// Catppuccin Mocha text and base: the emulator's default colours until a client reports its own.
pub const DEFAULT_FG: Rgb = Rgb {
    r: 0xcd,
    g: 0xd6,
    b: 0xf4,
};
pub const DEFAULT_BG: Rgb = Rgb {
    r: 0x1e,
    g: 0x1e,
    b: 0x2e,
};
const DRAIN_LIMIT: usize = 64;
// A child that exits within two seconds of its start did not run: two seconds is more than
// an ordinary shell needs to reach its prompt, and far less than the shortest session a
// person would open, so it separates a bad `terminal.shell` or a broken rc file from a shell
// somebody used and left. Three replacements ride out a transient failure - a lock held for
// a moment, a mount that was not ready - while capping one broken workspace at four process
// starts in total, after which the fourth pane is kept on screen with its exit status and
// the workspace stops replacing it.
const IMMEDIATE_EXIT: Duration = Duration::from_secs(2);
const MAX_IMMEDIATE_RESPAWNS: u8 = 3;

pub struct Core {
    pub model: Model,
    pub panes: HashMap<PaneId, PaneRuntime>,
    pub clients: HashMap<ClientId, ClientConn>,
    subscribers: Vec<(Vec<String>, mpsc::Sender<Event>)>,
    pub config: LoadedConfig,
    pub deps: CoreDeps,
    socket_path: PathBuf,
    state_dir: PathBuf,
    started_at: String,
    core_tx: mpsc::Sender<CoreMsg>,
    persist_tx: mpsc::Sender<StateFile>,
    pending_events: Vec<Event>,
    /// The model as of the end of the last batch, for readers outside the core. The core is
    /// the only writer and every reader gets a clone, so nothing outside holds a reference
    /// into the state the core owns.
    snapshot: Arc<Mutex<Model>>,
    /// Published beside `snapshot`: see `ServerHandle::pane_sizes`.
    pane_sizes: Arc<Mutex<HashMap<PaneId, Size>>>,
    /// Set whenever something a frame shows may have changed.
    view_dirty: bool,
    last_minute: Option<String>,
    stopping: bool,
    /// When each live pane's process started, for the immediate-exit guard below.
    pane_started_at: HashMap<PaneId, Instant>,
    /// Consecutive immediate exits of a workspace's last pane. Cleared as soon as any pane
    /// in that workspace survives `IMMEDIATE_EXIT`.
    immediate_exits: HashMap<WorkspaceId, u8>,
    /// Workspaces whose exited pane is kept rather than replaced.
    respawn_blocked: HashSet<WorkspaceId>,
}

impl Core {
    pub fn new(
        opts: ServerOptions,
        core_tx: mpsc::Sender<CoreMsg>,
        persist_tx: mpsc::Sender<StateFile>,
        state_file: &Path,
        snapshot: Arc<Mutex<Model>>,
        pane_sizes: Arc<Mutex<HashMap<PaneId, Size>>>,
    ) -> anyhow::Result<Core> {
        let started_at = opts.deps.clock.now().to_rfc3339();
        let mut model = match std::fs::read_to_string(state_file) {
            Ok(text) => match state_file::parse(&text).and_then(state_file::restore) {
                Ok(m) => m,
                Err(e) => {
                    // Move the refused file somewhere the writer never rotates. `write_atomic`
                    // renames the current file to `.bak` on every write, so a refused state
                    // file left in place survives exactly one structure change: the first
                    // write puts it in `.bak` and the second write puts the near-empty
                    // replacement over it. Two new tabs and the author's real state is gone.
                    let kept = crate::persist::with_suffix(state_file, ".rejected");
                    match std::fs::rename(state_file, &kept) {
                        Ok(()) => tracing::error!(
                            "{e}; the file that was refused is kept at {}; starting with an empty model",
                            kept.display()
                        ),
                        Err(move_failed) => tracing::error!(
                            "{e}; it could not be moved aside ({move_failed}), so it will be overwritten; starting with an empty model"
                        ),
                    }
                    Model::new(opts.deps.id_seed)
                }
            },
            Err(_) => Model::new(opts.deps.id_seed),
        };
        model.reseed(opts.deps.id_seed);
        // Architecture spec section 5: a pane whose directory is gone comes back in the
        // workspace path. Record the fallback so state.json stops naming a missing directory.
        let fallbacks: Vec<(PaneId, PathBuf)> = model
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .flat_map(|w| {
                w.tabs.iter().flat_map(move |t| {
                    t.layout
                        .panes()
                        .into_iter()
                        .map(move |pn| (pn.id.clone(), pn.cwd.clone(), w.path.clone()))
                })
            })
            .filter(|(_, cwd, _)| !cwd.is_dir())
            .map(|(id, _, ws_path)| (id, ws_path))
            .collect();
        for (pane, path) in fallbacks {
            tracing::info!(pane = %pane, "saved directory is gone; using the workspace path {}", path.display());
            model.set_pane_facts(
                &pane,
                PaneFacts {
                    cwd: Some(path),
                    ..Default::default()
                },
            );
        }
        if model.projects.is_empty() {
            // A fresh model has every id free, so this cannot exhaust the id space.
            model
                .add_folder_project(opts.project_root.clone())
                .expect("a fresh model cannot exhaust the id space");
        }
        let mut core = Core {
            model,
            panes: HashMap::new(),
            clients: HashMap::new(),
            subscribers: Vec::new(),
            config: opts.config,
            deps: opts.deps,
            socket_path: opts.socket_path,
            state_dir: opts.state_dir,
            started_at,
            core_tx,
            persist_tx,
            pending_events: Vec::new(),
            snapshot,
            pane_sizes,
            view_dirty: true,
            last_minute: None,
            stopping: false,
            pane_started_at: HashMap::new(),
            immediate_exits: HashMap::new(),
            respawn_blocked: HashSet::new(),
        };
        core.ensure_every_workspace_has_a_tab();
        for pane in core.model.all_pane_ids() {
            core.spawn_pane(&pane, Size { cols: 80, rows: 24 });
        }
        core.pending_events.push(Event::ServerStarted {
            version: domux_core::VERSION.into(),
            socket: core.socket_path.clone(),
        });
        Ok(core)
    }

    fn ensure_every_workspace_has_a_tab(&mut self) {
        let empty: Vec<(domux_core::ids::WorkspaceId, PathBuf)> = self
            .model
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .filter(|w| w.tabs.is_empty())
            .map(|w| (w.id.clone(), w.path.clone()))
            .collect();
        for (ws, path) in empty {
            match self.model.create_tab(&ws, path) {
                Ok((tab, _, events)) => {
                    self.pending_events.extend(events);
                    self.seat_stranded_clients(&ws, &tab);
                }
                Err(e) => tracing::error!("could not create a tab in workspace {ws}: {e}"),
            }
        }
    }

    /// Moves every client of `ws` whose tab the model no longer holds onto `tab`.
    ///
    /// `close_tab` moves a client to the tab that took the closed one's place, and there is
    /// none when the workspace's last tab closes: the client is then pointing at a tab that
    /// is gone, which no frame and no view method can answer for. The replacement tab this
    /// workspace just got is that place.
    fn seat_stranded_clients(&mut self, ws: &domux_core::ids::WorkspaceId, tab: &TabId) {
        let stranded: Vec<ClientId> = self
            .model
            .clients
            .iter()
            .filter(|c| &c.workspace == ws && self.model.tab(&c.tab).is_none())
            .map(|c| c.id.clone())
            .collect();
        for client in stranded {
            match self.model.select_tab(&client, tab) {
                Ok(events) => self.pending_events.extend(events),
                Err(e) => tracing::error!("could not seat client {client} on tab {tab}: {e}"),
            }
        }
    }

    /// Starts the PTY and emulator for a pane the Model already holds.
    pub fn spawn_pane(&mut self, pane: &PaneId, size: Size) {
        let Some(loc) = self.model.pane_location(pane) else {
            return;
        };
        let cwd = self
            .model
            .pane(pane)
            .map(|p| p.cwd.clone())
            .unwrap_or_else(|| PathBuf::from("/"));
        let (fg, bg) = self.default_colors();
        let emulator = match new_pane_emulator(size, self.config.config.terminal.scrollback, fg, bg)
        {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("emulator: {e}");
                return;
            }
        };
        let env = vec![
            ("DOMUX_PANE".to_string(), pane.to_string()),
            ("DOMUX_TAB".to_string(), loc.tab.to_string()),
            ("DOMUX_WORKSPACE".to_string(), loc.workspace.to_string()),
            ("DOMUX_PROJECT".to_string(), loc.project.to_string()),
            (
                "DOMUX_SOCKET".to_string(),
                self.socket_path.display().to_string(),
            ),
            // Which shell the pane runs, and the shell it says it is running. The spawner
            // reads this to run a login shell (see `SpawnRequest::command`), and a program
            // inside the pane that asks `$SHELL` gets the shell it is actually in rather
            // than whatever the server was started from.
            (
                "SHELL".to_string(),
                self.config.config.terminal.shell_or_default(),
            ),
        ];
        let req = SpawnRequest {
            pane: pane.clone(),
            // Empty: the shell, run as a terminal would run it.
            command: Vec::new(),
            cwd,
            env,
            size,
            term: PANE_TERM.into(),
        };
        match self.deps.spawner.spawn(req, self.core_tx.clone()) {
            Ok(pty) => {
                let pid = pty.pid();
                self.panes
                    .insert(pane.clone(), PaneRuntime::new(pane.clone(), emulator, pty));
                self.pane_started_at.insert(pane.clone(), Instant::now());
                self.model.set_pane_facts(
                    pane,
                    PaneFacts {
                        pid,
                        ..Default::default()
                    },
                );
                // Ask the inspector now rather than waiting for the next tick. A pane's box
                // shows its command, and up to a second of an untitled box is up to a
                // second of a frame that does not yet say what is running (principle 8).
                let observed = self.panes.get(pane).map(|rt| self.observe_pane(rt));
                if let Some(facts) = observed {
                    self.model.set_pane_facts(pane, facts);
                }
            }
            Err(e) => tracing::error!("spawn pane {pane}: {e}"),
        }
        self.view_dirty = true;
    }

    fn default_colors(&self) -> (Rgb, Rgb) {
        let recent = self
            .model
            .most_recent_client()
            .and_then(|id| self.model.client(&id).map(|c| c.caps.clone()));
        match recent {
            Some(caps) => (
                caps.default_fg.unwrap_or(DEFAULT_FG),
                caps.default_bg.unwrap_or(DEFAULT_BG),
            ),
            None => (DEFAULT_FG, DEFAULT_BG),
        }
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<CoreMsg>) {
        loop {
            let Some(first) = rx.recv().await else { break };
            self.handle(first);
            for _ in 0..DRAIN_LIMIT {
                match rx.try_recv() {
                    Ok(msg) => self.handle(msg),
                    Err(_) => break,
                }
            }
            self.after_batch();
            if self.stopping {
                break;
            }
        }
        self.shutdown();
    }

    fn handle(&mut self, msg: CoreMsg) {
        match msg {
            CoreMsg::PaneOutput { pane, bytes } => {
                let bell = match self.panes.get_mut(&pane) {
                    Some(p) => {
                        p.feed(&bytes);
                        p.emulator.take_bell()
                    }
                    None => false,
                };
                if bell {
                    self.bell(&pane);
                }
            }
            CoreMsg::PaneExited { pane, status } => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    let status = status.or_else(|| p.pty.exit_status());
                    p.exited = Some(status);
                    self.pending_events.push(Event::PaneExited {
                        pane: pane.clone(),
                        status,
                    });
                    self.view_dirty = true;
                }
            }
            CoreMsg::ClientConnected { hello, tx, reply } => {
                let _ = reply.send(self.attach(hello, tx));
            }
            CoreMsg::ClientInput { client, msg } => self.client_input(client, msg),
            CoreMsg::ClientGone { client } => self.detach(&client, None),
            CoreMsg::Api { request, reply } => {
                let response = self.api(request);
                let _ = reply.send(response);
            }
            CoreMsg::Subscribe { filter, tx } => self.subscribers.push((filter, tx)),
            CoreMsg::Tick => self.tick(),
            CoreMsg::Snapshot { reply } => {
                let _ = reply.send(self.model.clone());
            }
            CoreMsg::Shutdown => self.stopping = true,
        }
    }

    fn attach(&mut self, hello: Hello, tx: mpsc::Sender<ServerMsg>) -> Result<ClientId, String> {
        if hello.version != domux_core::VERSION
            || hello.protocol != domux_core::proto::PROTOCOL_VERSION
        {
            return Err(format!(
                "the server is {} {} and this client is {}; run {} server restart",
                domux_core::names::PRODUCT_NAME,
                domux_core::VERSION,
                hello.version,
                domux_core::names::BIN_NAME
            ));
        }
        let id = ClientId(self.model.next_id("c").map_err(|e| e.to_string())?);
        // Both ids come from the state file, so both can name something the file no longer
        // holds. A client that cannot be seated cannot attach at all, so a stale id here
        // would refuse every client and leave the server unusable with no way back but
        // deleting the file. Fall back to a workspace and a tab that exist.
        let workspace = self
            .model
            .last_workspace
            .clone()
            .filter(|w| self.model.workspace(w).is_some())
            .or_else(|| self.model.first_workspace())
            .ok_or("the server has no workspace")?;
        let ws = self
            .model
            .workspace(&workspace)
            .ok_or("the server has no workspace")?;
        let tab = ws
            .last_tab
            .clone()
            .filter(|t| ws.tabs.iter().any(|x| &x.id == t))
            .or_else(|| ws.tabs.first().map(|t| t.id.clone()))
            .ok_or("the workspace has no tab")?;
        let focused = self
            .model
            .tab(&tab)
            .map(|t| t.focused.clone())
            .ok_or("the tab has no pane")?;
        let view = ClientView {
            id: id.clone(),
            size: Size {
                cols: hello.cols,
                rows: hello.rows,
            },
            caps: hello.caps.clone(),
            workspace: workspace.clone(),
            tab,
            focus: Focus::Pane(focused),
            sidebar_open: false,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            filtering: false,
            input: domux_core::model::TextInput::new(""),
            overlay_under: None,
            pill: None,
        };
        self.pending_events.extend(self.model.attach_client(view));
        let _ = tx.try_send(ServerMsg::Welcome {
            client: id.clone(),
            version: domux_core::VERSION.into(),
        });
        self.clients.insert(
            id.clone(),
            ClientConn::new(id.clone(), tx, hello.caps, hello.cols, hello.rows),
        );
        // A client attaching after the guard tripped sees the same hint as one that watched
        // it trip, rather than a workspace with a dead pane and no reason given.
        if self.respawn_blocked.contains(&workspace) {
            self.set_shell_failure_hint(&workspace);
        }
        self.view_dirty = true;
        Ok(id)
    }

    /// Removes a client. `reason` is sent when the server initiated the detach.
    pub fn detach(&mut self, client: &ClientId, reason: Option<&str>) {
        if let Some(conn) = self.clients.remove(client) {
            if let Some(reason) = reason {
                let _ = conn.tx.try_send(ServerMsg::Detached {
                    reason: reason.into(),
                });
            }
        }
        self.pending_events.extend(self.model.detach_client(client));
        self.view_dirty = true;
    }

    fn client_input(&mut self, client: ClientId, msg: ClientMsg) {
        if !self.clients.contains_key(&client) {
            return;
        }
        self.model.touch_client(&client);
        match msg {
            ClientMsg::Hello(_) => {}
            ClientMsg::Key(key) => {
                self.clear_action_hint(&client);
                self.key(&client, key);
            }
            ClientMsg::Paste(text) => {
                if let Some(pane) = self.focused_pane(&client) {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        let mut out = Vec::new();
                        p.emulator.encode_paste(&text, &mut out);
                        p.write(&out);
                    }
                }
            }
            ClientMsg::Resize { cols, rows } => {
                if let Some(view) = self.model.client_mut(&client) {
                    view.size = Size { cols, rows };
                }
                if let Some(conn) = self.clients.get_mut(&client) {
                    conn.resize(cols, rows);
                }
                self.view_dirty = true;
            }
            ClientMsg::Focus(focused) => {
                if let Some(pane) = self.focused_pane(&client) {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        let mut out = Vec::new();
                        p.emulator.encode_focus(focused, &mut out);
                        p.write(&out);
                    }
                }
            }
            ClientMsg::Detach => self.detach(&client, Some("detached")),
            ClientMsg::ClipboardFailed(reason) => {
                if let Some(conn) = self.clients.get_mut(&client) {
                    conn.hint = Some(Hint::action(format!("clipboard failed: {reason}")));
                }
                self.view_dirty = true;
            }
        }
    }

    /// One key, routed by `input::route_key`. Every key gets a frame: the chord indicator
    /// appearing, an action's result, a hint cleared or replaced (principle 8).
    fn key(&mut self, client: &ClientId, key: domux_term::KeyEvent) {
        let _ = crate::input::route_key(self, client, key);
        self.view_dirty = true;
    }

    /// Runs a keymap action through the same dispatcher the API uses.
    ///
    /// A failed action's message shows in the clock's place until the next key, so a key
    /// that cannot do what it says still answers (principle 8) and the message names the
    /// object and the next action (principle 9).
    pub fn run_action(&mut self, client: &ClientId, action: &Action) {
        match Method::from_action(action) {
            Ok(method) => {
                if let Some(confirm) = self.confirmation_for(client, &method) {
                    if let Some(view) = self.model.client_mut(client) {
                        view.overlay = Some(Overlay::Confirm(confirm));
                        view.focus = Focus::Region(RegionKind::Overlay);
                    }
                    self.view_dirty = true;
                    return;
                }
                if let Err(e) = self.dispatch(method, Some(client.clone())) {
                    tracing::info!(client = %client, action = %action, "{}", e.message);
                    if let Some(conn) = self.clients.get_mut(client) {
                        conn.hint = Some(Hint::action(e.message));
                    }
                }
            }
            Err(e) => {
                tracing::warn!(action = %action, "{}", e.message);
                if let Some(conn) = self.clients.get_mut(client) {
                    conn.hint = Some(Hint::action(e.message));
                }
            }
        }
    }

    /// The question a key has to ask before it acts, if any. Closing a tab takes every
    /// process in it with it and nothing brings them back, so the key asks first
    /// (principle 10). The API method does not ask: a caller that sent `tab.close` has
    /// already decided, and a script is not who this protects.
    ///
    /// The tab is resolved here rather than in the handler so the question can name the tab
    /// the close would actually take, which is not always the current one: a binding may
    /// carry its own target, as `tab.close 2`.
    fn confirmation_for(&self, client: &ClientId, method: &Method) -> Option<ConfirmKind> {
        let Method::TabClose(p) = method else {
            return None;
        };
        let view = self.model.client(client)?;
        let tab = match p.tab.as_deref() {
            Some(t) => self.model.resolve_tab(&view.workspace, t).ok()?,
            None => view.tab.clone(),
        };
        Some(ConfirmKind::CloseTab(tab))
    }

    /// Clears the notice a failed action left, so it stands until the next key and no
    /// longer. A system notice is not one of those: it describes a state that is still true,
    /// and typing does not make it untrue, so it is left for whoever set it to withdraw when
    /// the state ends. Which is which is the hint's kind, not its text: a notice that names
    /// the configured shell stops matching a freshly generated one the moment the config
    /// changes, and a message must not depend on being reproducible to survive a keystroke.
    fn clear_action_hint(&mut self, client: &ClientId) {
        if let Some(conn) = self.clients.get_mut(client) {
            if conn
                .hint
                .as_ref()
                .is_some_and(|h| h.kind == HintKind::Action)
            {
                conn.hint = None;
            }
        }
    }

    pub fn focused_pane(&self, client: &ClientId) -> Option<PaneId> {
        let view = self.model.client(client)?;
        match &view.focus {
            Focus::Pane(_) | Focus::Region(_) => {
                self.model.tab(&view.tab).map(|t| t.focused.clone())
            }
        }
    }

    fn api(&mut self, request: Request) -> Response {
        let id = request.id.clone();
        let method = match Method::from_request(&request.method, request.params) {
            Ok(m) => m,
            Err(e) => return Response::err(id, e),
        };
        let client = param_client(&method).or_else(|| self.model.most_recent_client());
        match self.dispatch(method, client) {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(id, e),
        }
    }

    /// The one entry point for methods, from the API and from keys.
    pub fn dispatch(
        &mut self,
        method: Method,
        client: Option<ClientId>,
    ) -> Result<serde_json::Value, ApiError> {
        let mut ctx = Ctx {
            model: &mut self.model,
            panes: &mut self.panes,
            clients: &mut self.clients,
            config: &mut self.config,
            deps: &self.deps,
            core_tx: &self.core_tx,
            socket_path: &self.socket_path,
            state_dir: &self.state_dir,
            started_at: &self.started_at,
            client,
            events: Vec::new(),
            stop_requested: false,
            view_dirty: false,
            pending_spawns: Vec::new(),
            pending_kills: Vec::new(),
            detach_clients: Vec::new(),
            release_respawn_blocks: false,
        };
        let result = api::dispatch(method, &mut ctx);
        let events = std::mem::take(&mut ctx.events);
        let spawns = std::mem::take(&mut ctx.pending_spawns);
        let kills = std::mem::take(&mut ctx.pending_kills);
        let detaches = std::mem::take(&mut ctx.detach_clients);
        let stop = ctx.stop_requested;
        // Read out of `ctx` before it is dropped: the borrow of `self` ends with it.
        let view_dirty = ctx.view_dirty;
        let release_blocks = ctx.release_respawn_blocks;
        drop(ctx);
        if stop {
            self.stopping = true;
        }
        self.view_dirty |= view_dirty;
        self.pending_events.extend(events);
        // Before the side effects: a workspace whose block has just been lifted takes its
        // replacement pane from the invariant below like any other.
        if release_blocks {
            self.release_respawn_blocks();
        }
        self.apply_side_effects(spawns, kills, detaches);
        result
    }

    /// Does what a handler recorded but could not do itself: kill the PTYs of the panes it
    /// closed, start the panes it created, drop the clients it detached, and leave no
    /// workspace without a tab.
    ///
    /// Kills come before spawns so a close-and-replace frees its process before the
    /// replacement starts, and `sync_pane_sizes` comes last so a new PTY is at the size the
    /// smallest client actually draws before its program has printed anything.
    pub fn apply_side_effects(
        &mut self,
        spawns: Vec<PaneId>,
        kills: Vec<PaneId>,
        detaches: Vec<ClientId>,
    ) {
        let acted = !spawns.is_empty() || !kills.is_empty() || !detaches.is_empty();
        for pane in kills {
            if let Some(mut rt) = self.panes.remove(&pane) {
                rt.pty.kill();
            }
            self.pane_started_at.remove(&pane);
        }
        for pane in spawns {
            let size = self.provisional_size(&pane);
            self.spawn_pane(&pane, size);
        }
        for client in detaches {
            self.detach(&client, Some("detached"));
        }
        let before = self.model.all_pane_ids();
        self.ensure_every_workspace_has_a_tab();
        let created: Vec<PaneId> = self
            .model
            .all_pane_ids()
            .into_iter()
            .filter(|p| !before.contains(p))
            .collect();
        let replaced = !created.is_empty();
        for new in created {
            if self.replacement_is_blocked(&new) {
                continue;
            }
            let size = self.provisional_size(&new);
            self.spawn_pane(&new, size);
        }
        self.sync_pane_sizes();
        // Only when something happened. A read-only method reaches here with three empty
        // lists and nothing to replace, and must leave a clean view clean.
        if acted || replaced {
            self.view_dirty = true;
        }
    }

    /// Whether the respawn guard holds the workspace this replacement pane landed in.
    ///
    /// The one place the bound is enforced, because this is the one place a pane is
    /// replaced: `close_exited_panes` counts the exits, but Enter on a dead pane, `pane.close`
    /// and `tab.close` all reach a replacement through the workspace invariant above without
    /// passing through the counter, and each of those was an unbounded way to start shells
    /// after the bound had tripped. The pane itself stays in the model - a workspace without
    /// a tab is a state no frame and no view method can answer for - it simply gets no
    /// process until `config.reload` says the shell is fixed, and the notice says so.
    fn replacement_is_blocked(&mut self, pane: &PaneId) -> bool {
        let Some(workspace) = self
            .model
            .pane_location(pane)
            .map(|location| location.workspace)
        else {
            return false;
        };
        if !self.respawn_blocked.contains(&workspace) {
            return false;
        }
        self.set_shell_failure_hint(&workspace);
        true
    }

    /// The size the smallest client on the pane's tab will give it, so its PTY starts at the
    /// size it will be drawn at rather than at a default it is resized away from one batch
    /// later. The same arithmetic as `sync_pane_sizes`, which settles it either way.
    fn provisional_size(&self, pane: &PaneId) -> Size {
        let fallback = Size { cols: 80, rows: 24 };
        let Some(loc) = self.model.pane_location(pane) else {
            return fallback;
        };
        let Some(tab) = self.model.tab(&loc.tab) else {
            return fallback;
        };
        let area = render::workpanel_area(render::smallest_size(&self.model, &loc.tab, fallback));
        domux_core::model::layout::solve(&tab.layout, area, tab.zoomed.as_ref())
            .into_iter()
            .find(|(p, _)| p == pane)
            .map(|(_, r)| Size {
                cols: r.width.saturating_sub(2).max(1),
                rows: r.height.saturating_sub(2).max(1),
            })
            .unwrap_or(fallback)
    }

    /// What the process inspector and the emulator say about one pane right now. Each field
    /// is `None` when nothing answered, so `set_pane_facts` leaves that fact alone rather
    /// than clearing it.
    fn observe_pane(&self, pane: &PaneRuntime) -> PaneFacts {
        let fg = self.deps.inspector.foreground(pane.pty.raw_fd());
        let cwd = pane
            .emulator
            .cwd()
            .or_else(|| fg.as_ref().and_then(|f| self.deps.inspector.cwd_of(f.pid)));
        PaneFacts {
            command: fg.as_ref().map(|f| f.name.clone()),
            pid: fg.as_ref().map(|f| f.pid),
            cwd,
            title: pane.emulator.title(),
        }
    }

    fn tick(&mut self) {
        let mut changed = false;
        for (id, pane) in &self.panes {
            let facts = self.observe_pane(pane);
            if let Some(current) = self.model.pane(id) {
                if (facts.command.is_some() && facts.command != current.command)
                    || (facts.cwd.is_some() && facts.cwd.as_ref() != Some(&current.cwd))
                    || (facts.title.is_some() && facts.title != current.title)
                {
                    changed = true;
                }
            }
            self.model.set_pane_facts(id, facts);
        }
        let minute = self.deps.clock.now().format("%H:%M").to_string();
        if self.last_minute.as_ref() != Some(&minute) {
            self.last_minute = Some(minute);
            changed = true;
        }
        if changed {
            self.view_dirty = true;
            self.persist();
        }
    }

    fn bell(&mut self, pane: &PaneId) {
        let Some(loc) = self.model.pane_location(pane) else {
            return;
        };
        for view in &self.model.clients {
            if view.tab == loc.tab {
                if let Some(conn) = self.clients.get(&view.id) {
                    let _ = conn.tx.try_send(ServerMsg::Bell);
                }
            }
        }
    }

    fn after_batch(&mut self) {
        self.reset_respawn_guards_for_surviving_panes();
        self.close_exited_panes();
        self.publish_events();
        self.sync_pane_sizes();
        // Before the frames, not after: `render` does not touch the model, and publishing
        // first means a reader that has seen a frame is reading a model at least as new as
        // that frame. The other order leaves a window in which a test waits for a frame,
        // asks for the model and gets the one from before the batch.
        //
        // Sizes before the model, for the same reason one step smaller: a reader holding a
        // model from this batch then finds a size for every pane in it, rather than a pane
        // whose size has not been published yet.
        *self.pane_sizes.lock().unwrap() = self
            .panes
            .iter()
            .map(|(id, p)| (id.clone(), p.size()))
            .collect();
        *self.snapshot.lock().unwrap() = self.model.clone();
        self.render();
    }

    fn close_exited_panes(&mut self) {
        if self.config.config.terminal.remain_on_exit {
            return;
        }
        let exited: Vec<PaneId> = self
            .panes
            .iter()
            .filter(|(_, p)| p.exited.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for pane in exited {
            let Some(workspace) = self
                .model
                .pane_location(&pane)
                .map(|location| location.workspace)
            else {
                continue;
            };
            // Already tripped: the pane stays as it is, so the same exit is not counted
            // again on every later batch.
            if self.respawn_blocked.contains(&workspace) {
                continue;
            }
            // Only the workspace's last pane counts. Closing any other one leaves the
            // workspace with a tab, so nothing replaces it and there is no loop to bound.
            if self.is_last_pane_in_workspace(&pane, &workspace)
                && self
                    .pane_started_at
                    .get(&pane)
                    .is_some_and(|started| started.elapsed() < IMMEDIATE_EXIT)
            {
                let exits = self.immediate_exits.get(&workspace).copied().unwrap_or(0);
                if exits >= MAX_IMMEDIATE_RESPAWNS {
                    self.respawn_blocked.insert(workspace.clone());
                    self.set_shell_failure_hint(&workspace);
                    continue;
                }
                // `saturating_add` rather than `+`: the branch above caps `exits` at
                // `MAX_IMMEDIATE_RESPAWNS`, so this cannot reach 255, and total arithmetic
                // keeps that a fact about the counter rather than about the guard above it.
                self.immediate_exits
                    .insert(workspace.clone(), exits.saturating_add(1));
            }
            self.close_pane(&pane);
        }
    }

    /// Whether `pane` is the only pane the workspace has, which is what makes closing it
    /// close the workspace's last tab and so bring a replacement.
    fn is_last_pane_in_workspace(&self, pane: &PaneId, workspace: &WorkspaceId) -> bool {
        self.model.workspace(workspace).is_some_and(|ws| {
            ws.tabs
                .iter()
                .flat_map(|tab| tab.layout.pane_ids())
                .eq(std::iter::once(pane.clone()))
        })
    }

    /// A workspace with a pane that has been alive longer than `IMMEDIATE_EXIT` is working,
    /// so it gets its full allowance back: someone who fixes their rc file and starts a
    /// shell that lives is not held to the old count until the server restarts.
    fn reset_respawn_guards_for_surviving_panes(&mut self) {
        let survived: HashSet<WorkspaceId> = self
            .panes
            .iter()
            .filter(|(pane, runtime)| {
                runtime.exited.is_none()
                    && self
                        .pane_started_at
                        .get(*pane)
                        .is_some_and(|started| started.elapsed() >= IMMEDIATE_EXIT)
            })
            .filter_map(|(pane, _)| {
                self.model
                    .pane_location(pane)
                    .map(|location| location.workspace)
            })
            .collect();
        for workspace in survived {
            self.immediate_exits.remove(&workspace);
            if self.respawn_blocked.remove(&workspace) {
                self.clear_shell_failure_hint(&workspace);
            }
        }
    }

    fn set_shell_failure_hint(&mut self, workspace: &WorkspaceId) {
        let hint = self.shell_failure_hint();
        for view in self
            .model
            .clients
            .iter()
            .filter(|view| &view.workspace == workspace)
        {
            if let Some(conn) = self.clients.get_mut(&view.id) {
                conn.hint = Some(Hint::shell_failure(hint.clone()));
            }
        }
        self.view_dirty = true;
    }

    /// Clears only the hint this guard set, so a clipboard failure or another notice put
    /// there since is left alone. By kind: the stored text names the shell the guard tripped
    /// on, which a reload may since have changed.
    fn clear_shell_failure_hint(&mut self, workspace: &WorkspaceId) {
        for view in self
            .model
            .clients
            .iter()
            .filter(|view| &view.workspace == workspace)
        {
            if let Some(conn) = self.clients.get_mut(&view.id) {
                if conn
                    .hint
                    .as_ref()
                    .is_some_and(|h| h.kind == HintKind::ShellFailure)
                {
                    conn.hint = None;
                }
            }
        }
        self.view_dirty = true;
    }

    /// Names the shell that failed and the key that sets it, so the notice says what to fix
    /// rather than that something is wrong.
    fn shell_failure_hint(&self) -> String {
        let shell = self.config.config.terminal.shell_or_default();
        format!("shell {shell} exited immediately; set terminal.shell in domux.toml")
    }

    /// Enter on a pane whose child exited (`terminal.remain_on_exit`) closes it, which in a
    /// workspace of one pane brings a fresh tab and a fresh shell.
    ///
    /// A workspace the respawn guard has blocked gets the notice again instead. Retrying by
    /// hand is a reasonable thing to want, but Enter is a key a terminal repeats, so a retry
    /// on this key is a spawn storm on a held key. `config.reload` is the retry: it is the
    /// signal that the shell was fixed rather than that the key was pressed again.
    pub fn close_exited_pane(&mut self, pane: &PaneId) {
        match self
            .model
            .pane_location(pane)
            .map(|location| location.workspace)
        {
            Some(workspace) if self.respawn_blocked.contains(&workspace) => {
                self.set_shell_failure_hint(&workspace)
            }
            _ => self.close_pane(pane),
        }
    }

    /// A reload is the "I have fixed it" signal, so every blocked workspace gets its whole
    /// allowance back, its notice withdrawn, and a process for the pane the block left
    /// without one. It is what makes the notice actionable (principle 9): naming
    /// `terminal.shell` is only useful if setting it has an effect short of a restart.
    ///
    /// Every blocked workspace, because a reload replaces the whole config and `terminal.shell`
    /// is one setting for all of them. Called only for a reload that loaded: a file that does
    /// not parse leaves the previous config in place, so nothing about the shell changed.
    fn release_respawn_blocks(&mut self) {
        for workspace in std::mem::take(&mut self.respawn_blocked) {
            self.immediate_exits.remove(&workspace);
            self.clear_shell_failure_hint(&workspace);
            let stopped: Vec<PaneId> = self
                .model
                .workspace(&workspace)
                .into_iter()
                .flat_map(|ws| ws.tabs.iter().flat_map(|tab| tab.layout.pane_ids()))
                .filter(|pane| !self.panes.contains_key(pane))
                .collect();
            for pane in stopped {
                let size = self.provisional_size(&pane);
                self.spawn_pane(&pane, size);
            }
        }
    }

    /// Kills the PTY and removes the pane from the Model. A workspace never ends up without a
    /// tab: when the last one closes a fresh tab with a shell replaces it.
    pub fn close_pane(&mut self, pane: &PaneId) {
        // Closing the tab's last pane closes the tab, and `close_pane` then names every
        // pane that went with it, so the kill list comes from the model rather than from
        // this one id.
        let kills = match self.model.close_pane(pane) {
            Ok((panes, _, events)) => {
                self.pending_events.extend(events);
                panes
            }
            Err(e) => {
                // The model does not hold it; the runtime may still, so it is still killed.
                tracing::debug!("close pane {pane}: {e}");
                vec![pane.clone()]
            }
        };
        self.apply_side_effects(Vec::new(), kills, Vec::new());
    }

    fn publish_events(&mut self) {
        if self.pending_events.is_empty() {
            return;
        }
        let events = std::mem::take(&mut self.pending_events);
        let structural = events.iter().any(|e| {
            !matches!(
                e,
                Event::ClientAttached { .. }
                    | Event::ClientDetached { .. }
                    | Event::ServerStarted { .. }
                    | Event::TabSelected { .. }
            )
        });
        self.subscribers.retain(|(filter, tx)| {
            for e in &events {
                if e.matches(filter) && tx.try_send(e.clone()).is_err() {
                    return false;
                }
            }
            true
        });
        if structural {
            self.persist();
        }
    }

    fn persist(&self) {
        let snapshot = state_file::snapshot(&self.model, &self.deps.clock.now().to_rfc3339());
        let _ = self.persist_tx.try_send(snapshot);
    }

    /// Every pane viewed by a client that draws panes takes the size the smallest such client
    /// gives it. Panes without a drawing client keep their size.
    fn sync_pane_sizes(&mut self) {
        // One entry per tab with a client that draws panes, carrying the first such client's
        // size. A below-minimum client draws only the size notice, so it must not resize a PTY
        // nobody can see. When every client is below the minimum, leave the existing pane size
        // alone: a resize would churn its program for no visible result. `smallest_size` still
        // raises an absent current size to the minimum, rather than adopting a tiny screen.
        // rectangle itself comes from `render::smallest_size`, the same function the
        // renderer lays the boxes out with, so a pane's program and every client agree on
        // its size. The recorded size is only the fallback for a tab without another drawing
        // client, which cannot happen here: each entry was made from a drawing client.
        let mut viewed: Vec<(TabId, Size)> = Vec::new();
        for view in
            self.model.clients.iter().filter(|view| {
                view.size.cols >= render::MIN_COLS && view.size.rows >= render::MIN_ROWS
            })
        {
            if !viewed.iter().any(|(t, _)| t == &view.tab) {
                viewed.push((view.tab.clone(), view.size));
            }
        }
        let mut events = Vec::new();
        for (tab_id, fallback) in viewed {
            let size = render::smallest_size(&self.model, &tab_id, fallback);
            let Some(tab) = self.model.tab(&tab_id) else {
                continue;
            };
            let area = render::workpanel_area(size);
            for (pane, rect) in
                domux_core::model::layout::solve(&tab.layout, area, tab.zoomed.as_ref())
            {
                let inner = Size {
                    cols: rect.width.saturating_sub(2).max(1),
                    rows: rect.height.saturating_sub(2).max(1),
                };
                if let Some(rt) = self.panes.get_mut(&pane) {
                    if rt.size() != inner {
                        rt.resize(inner);
                        events.push(Event::PaneResized {
                            pane: pane.clone(),
                            cols: inner.cols,
                            rows: inner.rows,
                        });
                    }
                }
            }
        }
        if !events.is_empty() {
            self.pending_events.extend(events);
            self.publish_events();
        }
    }

    fn render(&mut self) {
        let any_dirty = self.panes.values().any(|p| p.dirty);
        if !self.view_dirty && !any_dirty {
            return;
        }
        for p in self.panes.values_mut() {
            if p.dirty {
                p.snapshot();
            }
        }
        for view in self.model.clients.clone() {
            let Some(conn) = self.clients.get_mut(&view.id) else {
                continue;
            };
            let input = RenderInput {
                model: &self.model,
                panes: &self.panes,
                view: &view,
                keymap: &self.config.keymap,
                now: self.deps.clock.now(),
                config_error: self.config.error.as_ref(),
                hint: conn.hint.as_ref(),
            };
            let (buffer, cursor) = render::compose(&input);
            conn.queue_frame(buffer, cursor);
        }
        self.view_dirty = false;
    }

    fn shutdown(mut self) {
        self.pending_events.push(Event::ServerStopping);
        self.publish_events();
        self.persist();
        for id in self.clients.keys().cloned().collect::<Vec<_>>() {
            self.detach(&id, Some(domux_core::proto::SERVER_STOPPED));
        }
        for (_, mut p) in self.panes.drain() {
            p.pty.kill();
        }
        // `self` drops here with the only `persist_tx`; the persistence task then writes the
        // last snapshot at once and exits, and `ServerHandle::stop` awaits it.
    }
}

/// The `client` parameter of a view method, when the request carried one.
fn param_client(method: &Method) -> Option<ClientId> {
    use Method::*;
    match method {
        ClientDetach(p) | Help(p) | FocusLeft(p) | FocusRight(p) | FocusUp(p) | FocusDown(p)
        | FocusLast(p) | FocusPane(p) => p.client.clone(),
        FocusRegion(p) => p.client.clone(),
        TabCreate(p) => p.client.clone(),
        TabRename(p) => p.client.clone(),
        TabClearName(p) | TabClose(p) | PaneList(p) => p.client.clone(),
        TabSelect(p) => p.client.clone(),
        PaneClose(p) | PaneFocus(p) | PaneZoom(p) | PaneCopyMode(p) => p.client.clone(),
        PaneSplit(p) => p.client.clone(),
        PaneResize(p) => p.client.clone(),
        PaneSendText(p) => p.client.clone(),
        PaneSendKey(p) => p.client.clone(),
        PaneRead(p) => p.client.clone(),
        ProjectAdd(p) => p.client.clone(),
        WorkspaceCreate(p) => p.client.clone(),
        WorkspaceRename(p) => p.client.clone(),
        WorkspaceFocus(p) => p.client.clone(),
        SwitcherOpen(p) | SwitcherClose(p) | SidebarToggle(p) | SidebarShow(p) | SidebarHide(p)
        | ListDown(p) | ListUp(p) | ListActivate(p) | ListFilter(p) => p.client.clone(),
        ServerInfo(_)
        | ServerStop(_)
        | EventsSubscribe(_)
        | ConfigReload(_)
        | TabList(_)
        | ProjectList(_)
        | ProjectRemove(_)
        | WorkspaceList(_)
        | WorkspaceClear(_)
        | WorkspaceDelete(_)
        | WorkspaceClearName(_)
        | WorkspaceResume(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::FakeSpawner;
    use crate::process::FakeInspector;
    use crate::{load_config, FixedClock};
    use domux_core::api::NoParams;

    fn core(dir: &Path) -> Core {
        let project = dir.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let (core_tx, _core_rx) = mpsc::channel(8);
        let (persist_tx, _persist_rx) = mpsc::channel(8);
        let opts = ServerOptions {
            socket_path: dir.join("s.sock"),
            state_dir: dir.join("state"),
            config: load_config(&dir.join("none.toml")),
            project_root: project,
            deps: CoreDeps {
                spawner: Arc::new(FakeSpawner::default()),
                inspector: Arc::new(FakeInspector::default()),
                clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
                id_seed: 7,
            },
        };
        Core::new(
            opts,
            core_tx,
            persist_tx,
            &dir.join("missing.json"),
            Arc::new(Mutex::new(Model::new(7))),
            Arc::new(Mutex::new(HashMap::new())),
        )
        .unwrap()
    }

    /// A read-only method must not compose a frame for every attached client. `dispatch`
    /// still runs `apply_side_effects`, which is why that call marks the view only when it
    /// actually killed, spawned, detached or replaced something.
    #[test]
    fn server_info_does_not_mark_the_view_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        core.view_dirty = false;
        core.dispatch(Method::ServerInfo(NoParams::default()), None)
            .unwrap();
        assert!(!core.view_dirty);
    }

    /// A failed action's notice stands until the next key. The shell-failure notice is not
    /// one of those: it names a state that is still true, and typing does not make it
    /// untrue, so a key must leave it where it is.
    #[test]
    fn a_key_clears_an_action_hint_and_keeps_the_shell_failure_notice() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let (tx, _rx) = mpsc::channel(64);
        let client = core
            .attach(
                Hello {
                    version: domux_core::VERSION.into(),
                    protocol: domux_core::proto::PROTOCOL_VERSION,
                    cols: 80,
                    rows: 24,
                    caps: Default::default(),
                },
                tx,
            )
            .unwrap();
        let press = || {
            ClientMsg::Key(domux_term::KeyEvent::press(
                domux_term::Key::Char('j'),
                domux_term::Mods::empty(),
            ))
        };
        core.clients.get_mut(&client).unwrap().hint = Some(Hint::action("no pane to the left"));
        core.client_input(client.clone(), press());
        assert_eq!(core.clients[&client].hint, None);
        let shell = core.shell_failure_hint();
        core.clients.get_mut(&client).unwrap().hint = Some(Hint::shell_failure(shell.clone()));
        core.client_input(client.clone(), press());
        assert_eq!(
            core.clients[&client].hint,
            Some(Hint::shell_failure(shell.clone()))
        );
        // And it is the kind that keeps it, not the text: a reload can change
        // `terminal.shell` while the workspace is still blocked, which leaves a stored
        // notice naming the old shell that no freshly generated string matches. Comparing
        // text cleared a notice that was still true on the very next key.
        core.config.config.terminal.shell = Some("/bin/other".into());
        assert_ne!(
            core.shell_failure_hint(),
            shell,
            "the notice text has moved"
        );
        core.client_input(client.clone(), press());
        assert_eq!(
            core.clients[&client].hint,
            Some(Hint::shell_failure(shell)),
            "a key cleared a notice whose text the config had moved under it"
        );
    }

    /// The register `only_the_expected_m2_methods_still_answer_unavailable` and
    /// `every_unavailable_arm_in_dispatch_is_listed_in_still_unbuilt` both check against:
    /// every method Task 4 declared but no task has given a handler yet. `workspace.resume`
    /// is the one deliberate exception, left for M3: when this reads exactly
    /// `[("workspace.resume", ..)]`, M2 has closed this class of gap.
    const STILL_UNBUILT: &[(&str, &str)] = &[
        ("project.list", "{}"),
        ("project.add", r#"{"path":"/x"}"#),
        ("project.remove", r#"{"project":"p"}"#),
        ("workspace.list", "{}"),
        ("workspace.create", r#"{"project":"p"}"#),
        ("workspace.clear", r#"{"workspace":"w"}"#),
        ("workspace.delete", r#"{"workspace":"w"}"#),
        ("workspace.rename", "{}"),
        ("workspace.clear_name", r#"{"workspace":"w"}"#),
        ("workspace.focus", r#"{"workspace":"w"}"#),
        ("workspace.resume", r#"{"workspace":"w"}"#),
        ("switcher.open", "{}"),
        ("switcher.close", "{}"),
        ("sidebar.toggle", "{}"),
        ("sidebar.show", "{}"),
        ("sidebar.hide", "{}"),
        ("list.down", "{}"),
        ("list.up", "{}"),
        ("list.activate", "{}"),
        ("list.filter", "{}"),
    ];

    /// Direction A of the register: implementing one of `STILL_UNBUILT` must fail this test
    /// until the implementer removes that method's line here and from the stub block in
    /// `api::dispatch`, so removal is forced rather than remembered.
    #[test]
    fn only_the_expected_m2_methods_still_answer_unavailable() {
        use domux_core::api::ErrorCode;
        let dir = tempfile::tempdir().unwrap();
        for (name, params) in STILL_UNBUILT {
            assert!(
                Method::NAMES.contains(name),
                "{name} is not a real method; fix STILL_UNBUILT"
            );
            let mut core = core(dir.path());
            let params = serde_json::from_str(params).unwrap();
            let method =
                Method::from_request(name, params).unwrap_or_else(|e| panic!("{name}: {e}"));
            let err = core
                .dispatch(method, None)
                .expect_err(&format!("{name} has a real handler now; remove it from STILL_UNBUILT and from the stub block in api::dispatch"));
            assert_eq!(err.code, ErrorCode::Unavailable, "{name}: {err}");
            assert!(err.message.ends_with("is not built yet"), "{name}: {err}");
        }
    }

    /// Direction B of the register: every arm in `api::dispatch` whose body answers
    /// `unavailable` with "is not built yet" must be listed in `STILL_UNBUILT` too, so a
    /// milestone cannot add a stub without declaring it - the half that dispatching each
    /// `STILL_UNBUILT` entry and checking the answer can never catch, because it only ever
    /// looks at the names already on that list.
    ///
    /// Scans the whole `match` in `dispatch`, not a marked-off block: an earlier version of
    /// this test looked only between a `// --- M2 stubs` comment and the first `=>` after
    /// it, which is exactly the shape of the *one* arm that block held at the time and
    /// nothing else. A stub written as its own arm anywhere else in the function - joined
    /// into no one's or-pattern, sitting next to `ServerInfo` rather than after the
    /// comment - was invisible to it. Proven by doing: adding such an arm and leaving it out
    /// of `STILL_UNBUILT` passed the old version clean. This one finds every arm regardless
    /// of where it sits or whether it stands alone or joins an or-pattern, because it is not
    /// looking for a comment - it is looking for the words the stub actually answers with,
    /// which is the property that actually matters.
    ///
    /// Reads both source files as text and cross-checks the identifiers against the wire
    /// names the `methods!` table gives them, rather than dispatching every method in
    /// `Method::NAMES` to see which answer `unavailable`: most of the other 28 have side
    /// effects (`server.stop` stops the server), so calling them just to observe an error
    /// code is not an option. This is the same move as
    /// `names::tests::nothing_outside_this_file_spells_the_binary_name`: pin the source text
    /// that has to stay in sync, not the behavior it happens to produce today.
    #[test]
    fn every_unavailable_arm_in_dispatch_is_listed_in_still_unbuilt() {
        let root = workspace_root();

        let table_src =
            std::fs::read_to_string(root.join("crates/domux-core/src/api.rs")).expect("read");
        let ident_to_wire = parse_methods_table(&table_src);

        let dispatch_src =
            std::fs::read_to_string(root.join("crates/domux-server/src/api/mod.rs")).expect("read");
        let signature =
            "pub fn dispatch(method: Method, ctx: &mut Ctx) -> Result<Value, ApiError> {";
        let body = function_body(&dispatch_src, signature);

        let mut from_dispatch: Vec<&str> = Vec::new();
        for (pattern, arm_body) in split_match_arms(body) {
            if !arm_body.contains("is not built yet") {
                continue;
            }
            for ident in pattern_identifiers(&pattern) {
                from_dispatch.push(
                    ident_to_wire
                        .get(&ident)
                        .unwrap_or_else(|| panic!("{ident} is not a method in the methods! table"))
                        .as_str(),
                );
            }
        }
        assert!(
            !from_dispatch.is_empty(),
            "no arm in dispatch answered \"is not built yet\"; the scan is broken"
        );
        from_dispatch.sort_unstable();
        from_dispatch.dedup();

        let mut from_still_unbuilt: Vec<&str> =
            STILL_UNBUILT.iter().map(|(name, _)| *name).collect();
        from_still_unbuilt.sort_unstable();

        assert_eq!(
            from_dispatch, from_still_unbuilt,
            "every arm in dispatch that answers \"is not built yet\" must be listed in \
             STILL_UNBUILT, and STILL_UNBUILT must list nothing else"
        );
    }

    /// Two levels above `crates/domux-server` is the workspace root, the same distance
    /// `names.rs`'s own version of this helper climbs from `crates/domux-core`.
    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels above crates/domux-server is the workspace root")
            .to_path_buf()
    }

    /// The `Ident = "wire.name": Params => Result` lines of the `methods!` table in
    /// `domux-core::api`, as an identifier to wire name map. The single source both
    /// `Method` and its `NAMES` are generated from, read as text so this test does not need
    /// its own copy of the mapping to drift against.
    fn parse_methods_table(src: &str) -> HashMap<String, String> {
        let marker = "methods! {";
        let after_marker = src.find(marker).expect("the methods! table") + marker.len();
        let body_start = src[after_marker..]
            .find('\n')
            .map(|i| after_marker + i + 1)
            .unwrap_or(after_marker);
        let rest = &src[body_start..];
        let body_end = rest
            .find("\n}\n")
            .expect("the methods! table's closing brace");
        let mut map = HashMap::new();
        for line in rest[..body_end].lines() {
            let line = line.trim();
            let Some((ident, remainder)) = line.split_once('=') else {
                continue;
            };
            let Some(name_start) = remainder.find('"') else {
                continue;
            };
            let after_quote = &remainder[name_start + 1..];
            let Some(name_end) = after_quote.find('"') else {
                continue;
            };
            map.insert(
                ident.trim().to_string(),
                after_quote[..name_end].to_string(),
            );
        }
        map
    }

    /// The text between the `{` that opens `signature`'s block and its matching `}`,
    /// brace-depth aware so a nested block never ends the scan early.
    fn function_body<'a>(src: &'a str, signature: &str) -> &'a str {
        let sig_pos = src
            .find(signature)
            .unwrap_or_else(|| panic!("{signature:?} not found in source"));
        let open = sig_pos + signature.len() - 1;
        assert_eq!(
            src.as_bytes()[open],
            b'{',
            "signature must end in its opening brace"
        );
        let close = matching_brace(src, open);
        &src[open + 1..close]
    }

    /// The index of the `}` that closes the `{` at byte offset `open`, skipping over string
    /// contents so a literal brace inside a message - `"{unbuilt} is not built yet"` has one
    /// of each - is never mistaken for real nesting.
    fn matching_brace(src: &str, open: usize) -> usize {
        let bytes = src.as_bytes();
        assert_eq!(bytes[open], b'{');
        let mut depth = 0i32;
        let mut i = open;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => i = skip_string(bytes, i),
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        panic!("unbalanced braces from offset {open}");
    }

    /// The index just past the closing `"` of the string starting at `at`, honoring `\"`.
    fn skip_string(bytes: &[u8], at: usize) -> usize {
        assert_eq!(bytes[at], b'"');
        let mut i = at + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'"' => return i + 1,
                _ => i += 1,
            }
        }
        bytes.len()
    }

    /// The index of the first `,` at bracket depth 0 from `start`, skipping string contents,
    /// or `None` when nothing remains but the match's last arm, which needs no trailing
    /// comma.
    fn find_top_level_comma(body: &str, start: usize) -> Option<usize> {
        let bytes = body.as_bytes();
        let mut depth = 0i32;
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => i = skip_string(bytes, i),
                b'(' | b'{' | b'[' => {
                    depth += 1;
                    i += 1;
                }
                b')' | b'}' | b']' => {
                    depth -= 1;
                    i += 1;
                }
                b',' if depth == 0 => return Some(i),
                _ => i += 1,
            }
        }
        None
    }

    /// Splits a `match { ... }` body's raw text into `(pattern, body)` pairs, one per arm,
    /// regardless of whether an arm stands alone or joins an or-pattern, and regardless of
    /// whether its body is a bare expression ending in `,` or a `{ ... }` block that may
    /// have no trailing comma at all (the shape every arm above the M2 stubs uses, and the
    /// shape the stubs themselves used until this test's blind spot was found). Finding the
    /// next arm's `=>` with a plain substring search is safe here because `dispatch`'s
    /// patterns never contain match guards or a literal `=>`; if that ever changes, this
    /// test starts failing loudly (a pattern's text bleeding into what looks like a body)
    /// rather than silently.
    fn split_match_arms(body: &str) -> Vec<(String, String)> {
        let mut arms = Vec::new();
        let mut pos = 0usize;
        while let Some(rel_arrow) = body[pos..].find("=>") {
            let arrow = pos + rel_arrow;
            let pattern = body[pos..arrow].to_string();
            let mut body_start = arrow + 2;
            while body_start < body.len() && (body.as_bytes()[body_start] as char).is_whitespace() {
                body_start += 1;
            }
            let (body_end, next_pos) = if body.as_bytes().get(body_start) == Some(&b'{') {
                let close = matching_brace(body, body_start);
                let mut next = close + 1;
                while next < body.len() && (body.as_bytes()[next] as char).is_whitespace() {
                    next += 1;
                }
                if body.as_bytes().get(next) == Some(&b',') {
                    next += 1;
                }
                (close + 1, next)
            } else {
                match find_top_level_comma(body, body_start) {
                    Some(c) => (c, c + 1),
                    None => (body.len(), body.len()),
                }
            };
            arms.push((pattern, body[body_start..body_end].to_string()));
            if next_pos <= pos {
                break;
            }
            pos = next_pos;
        }
        arms
    }

    /// The `Method` variant identifiers a match arm's pattern text names: line comments
    /// stripped (a pattern's text runs from the end of the previous arm, so it carries
    /// whatever comment sits between them, such as `// --- M2 stubs ... ---`), then split on
    /// `|` with each alternative's `(_)` or `(p)` stripped, so `A(_) | B(p)` gives
    /// `["A", "B"]` and a lone `A(_)` gives `["A"]`.
    fn pattern_identifiers(pattern: &str) -> Vec<String> {
        let uncommented: String = pattern
            .lines()
            .map(|line| line.split("//").next().unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        uncommented
            .split('|')
            .map(|alt| alt.trim())
            .filter(|alt| !alt.is_empty())
            .map(|alt| alt.split('(').next().unwrap_or(alt).trim().to_string())
            .collect()
    }

    /// The other side of the same flag, and the flag itself rather than the side-effect
    /// path: `pane.zoom` records no spawn, kill or detach, so `apply_side_effects` has
    /// nothing to act on and the frame is composed only because the handler asked for it.
    #[test]
    fn a_pane_zoom_marks_the_view_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let pane = core.model.all_pane_ids().first().cloned().unwrap();
        core.view_dirty = false;
        core.dispatch(
            Method::PaneZoom(domux_core::api::PaneTargetParams {
                pane: Some(pane.to_string()),
                client: None,
            }),
            None,
        )
        .unwrap();
        assert!(core.view_dirty);
    }
}
