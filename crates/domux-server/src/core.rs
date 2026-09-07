//! The core task: the one owner of the Model, every PaneRuntime and every ClientConn.

use crate::api::{self, Ctx};
use crate::client::ClientConn;
use crate::pane::{new_pane_emulator, PaneRuntime, SpawnRequest, PANE_TERM};
use crate::render::{self, RenderInput};
use crate::{CoreDeps, LoadedConfig, ServerOptions};
use domux_core::api::{ApiError, Event, Method, Request, Response};
use domux_core::ids::{ClientId, PaneId, TabId};
use domux_core::model::{ClientView, Focus, Model, PaneFacts};
use domux_core::proto::{ClientMsg, Hello, ServerMsg};
use domux_core::state_file::{self, StateFile};
use domux_term::{Emulator, Rgb, Size};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
                    tracing::error!("{e}; starting with an empty model");
                    Model::new(opts.deps.id_seed)
                }
            },
            Err(_) => Model::new(opts.deps.id_seed),
        };
        model.reseed(opts.deps.id_seed);
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
                Ok((_, _, events)) => self.pending_events.extend(events),
                Err(e) => tracing::error!("could not create a tab in workspace {ws}: {e}"),
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
        ];
        let req = SpawnRequest {
            pane: pane.clone(),
            command: vec![self.config.config.terminal.shell_or_default()],
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
                self.model.set_pane_facts(
                    pane,
                    PaneFacts {
                        pid,
                        ..Default::default()
                    },
                );
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
                "the server is domux {} and this client is {}; run domux2 server restart",
                domux_core::VERSION,
                hello.version
            ));
        }
        let id = ClientId(self.model.next_id("c").map_err(|e| e.to_string())?);
        let workspace = self
            .model
            .last_workspace
            .clone()
            .or_else(|| self.model.first_workspace())
            .ok_or("the server has no workspace")?;
        let ws = self
            .model
            .workspace(&workspace)
            .ok_or("the server has no workspace")?;
        let tab = ws
            .last_tab
            .clone()
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
            workspace,
            tab,
            focus: Focus::Pane(focused),
            sidebar_open: false,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
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
            ClientMsg::Key(key) => self.key(&client, key),
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
                    conn.hint = Some(format!("clipboard failed: {reason}"));
                }
                self.view_dirty = true;
            }
        }
    }

    /// Task 17 replaces this with `input::route_key`. Until then every key goes to the pane.
    fn key(&mut self, client: &ClientId, key: domux_term::KeyEvent) {
        if let Some(pane) = self.focused_pane(client) {
            if let Some(p) = self.panes.get_mut(&pane) {
                let mut out = Vec::new();
                p.emulator.encode_key(&key, &mut out);
                if !out.is_empty() {
                    p.write(&out);
                }
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

    /// The one entry point for methods, from the API and (Task 17) from keys.
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
        };
        let result = api::dispatch(method, &mut ctx);
        let events = std::mem::take(&mut ctx.events);
        if ctx.stop_requested {
            self.stopping = true;
        }
        self.pending_events.extend(events);
        self.view_dirty = true;
        result
    }

    fn tick(&mut self) {
        let mut changed = false;
        for (id, pane) in &self.panes {
            let fg = self.deps.inspector.foreground(pane.pty.raw_fd());
            let cwd = pane
                .emulator
                .cwd()
                .or_else(|| fg.as_ref().and_then(|f| self.deps.inspector.cwd_of(f.pid)));
            let facts = PaneFacts {
                command: fg.as_ref().map(|f| f.name.clone()),
                pid: fg.as_ref().map(|f| f.pid),
                cwd,
                title: pane.emulator.title(),
            };
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
            self.close_pane(&pane);
        }
    }

    /// Kills the PTY and removes the pane from the Model. A workspace never ends up without a
    /// tab: when the last one closes a fresh tab with a shell replaces it.
    pub fn close_pane(&mut self, pane: &PaneId) {
        if let Some(mut rt) = self.panes.remove(pane) {
            rt.pty.kill();
        }
        match self.model.close_pane(pane) {
            Ok((_, _, events)) => self.pending_events.extend(events),
            Err(e) => tracing::debug!("close pane {pane}: {e}"),
        }
        let before = self.model.all_pane_ids();
        self.ensure_every_workspace_has_a_tab();
        for new in self
            .model
            .all_pane_ids()
            .into_iter()
            .filter(|p| !before.contains(p))
        {
            self.spawn_pane(&new, Size { cols: 80, rows: 24 });
        }
        self.view_dirty = true;
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

    /// Every pane viewed by at least one client takes the size the smallest such client
    /// gives it. Panes nobody views keep their size.
    fn sync_pane_sizes(&mut self) {
        // One entry per viewed tab, carrying the first client's size seen for it. The
        // rectangle itself comes from `render::smallest_size`, the same function the
        // renderer lays the boxes out with, so a pane's program and every client agree on
        // its size. The recorded size is only the fallback for a tab no client views, which
        // cannot happen here: each entry was made from a client that views it.
        let mut viewed: Vec<(TabId, Size)> = Vec::new();
        for view in &self.model.clients {
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
                hint: conn.hint.as_deref(),
            };
            let (buffer, cursor) = render::compose(&input);
            if let Some(diff) = conn.take_frame(buffer, cursor) {
                let _ = conn.tx.try_send(ServerMsg::Frame(diff));
            }
        }
        self.view_dirty = false;
    }

    fn shutdown(mut self) {
        self.pending_events.push(Event::ServerStopping);
        self.publish_events();
        self.persist();
        for id in self.clients.keys().cloned().collect::<Vec<_>>() {
            self.detach(&id, Some("the server stopped"));
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
        ServerInfo(_) | ServerStop(_) | EventsSubscribe(_) | ConfigReload(_) | TabList(_) => None,
    }
}
