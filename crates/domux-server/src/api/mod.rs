//! One handler per method. A keybinding, a CLI subcommand and an API request all arrive
//! here as a `Method` and leave as a `Value` or an `ApiError`.

pub mod agent;
pub mod client;
pub mod config;
pub mod focus;
pub mod list;
pub mod pane;
pub mod project;
pub mod server;
pub mod sidebar;
pub mod switcher;
pub mod tab;
pub mod workspace;

use crate::client::ClientConn;
use crate::core::{CoreJob, CoreMsg};
use crate::facts::FactRegistry;
use crate::pane::PaneRuntime;
use crate::{CoreDeps, LoadedConfig};
use domux_core::api::{ApiError, Event, Method, PaneInfo};
use domux_core::ids::{ClientId, PaneId, TabId};
use domux_core::model::{Direction, Model, Rect};
use domux_term::Size;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::mpsc;

/// The workpanel a tab is laid out in when no client shows it. Nothing draws that tab, so
/// the number only has to be a sane terminal for geometry the caller asked about anyway.
const UNVIEWED_SIZE: Size = Size { cols: 80, rows: 24 };

/// Everything a handler may touch. Built by the core for one call.
pub struct Ctx<'a> {
    pub model: &'a mut Model,
    pub panes: &'a mut HashMap<PaneId, PaneRuntime>,
    pub clients: &'a mut HashMap<ClientId, ClientConn>,
    pub config: &'a mut LoadedConfig,
    pub deps: &'a CoreDeps,
    /// What domux observed. A handler reads a fact; it never fetches one, because a fetch
    /// shells out and a handler runs on the core task.
    pub facts: &'a FactRegistry,
    /// The caches and declarations the agent records need: working words, the transcript
    /// reader and the manifest registry. Not persisted.
    pub agents: &'a mut crate::agents::AgentsState,
    pub core_tx: &'a mpsc::Sender<CoreMsg>,
    pub socket_path: &'a PathBuf,
    pub state_dir: &'a PathBuf,
    pub started_at: &'a str,
    /// The view the call acts on: the pressing client for a keybinding, the `client` param
    /// or the most recent client for an API call.
    pub client: Option<ClientId>,
    /// True when a key press reached this handler rather than the control API. A
    /// destructive operation asks its question on the screen for a key and answers
    /// "Answer with --yes" for a caller that has a command line (interface spec 7.3).
    pub from_key: bool,
    pub events: Vec<Event>,
    pub stop_requested: bool,
    /// Set by a handler when it changes something a frame shows. A read-only method leaves
    /// it clear, so answering `server.info` or `pane.list` does not compose a frame for
    /// every attached client.
    pub view_dirty: bool,
    /// What the core must do once the handler returns. A handler is pure over the Model and
    /// the runtime maps; spawning a process, killing one and closing a connection are the
    /// core's, so they are recorded here rather than done in place.
    pub pending_spawns: Vec<PaneId>,
    pub pending_kills: Vec<PaneId>,
    pub detach_clients: Vec<ClientId>,
    /// Set by a config reload that loaded: the respawn guard's blocks are the core's, and a
    /// new config is the signal that the shell it tripped on may be fixed.
    pub release_respawn_blocks: bool,
    /// Work that shells out, for the core to run off its own task (decision record 0006).
    pub jobs: Vec<CoreJob>,
    /// What the jobs in flight have already chosen, keyed by `CoreJob::claim`. A handler
    /// that chooses something reads this so it does not choose what another call in flight
    /// has: see `Core::claims`. Handlers only read it; the core takes and releases the
    /// claims with the jobs themselves.
    pub claims: &'a HashSet<String>,
    /// Set beside a queued job by a handler whose answer that job carries: the caller is
    /// told when the job finishes, and the value this handler returns is not the answer.
    /// The first job queued is the one that carries it.
    pub defer_reply: bool,
}

impl Ctx<'_> {
    /// The client a view method acts on, or `not_found` when none is attached.
    pub fn view(&self) -> Result<ClientId, ApiError> {
        self.client
            .clone()
            .or_else(|| self.model.most_recent_client())
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "no client is attached; run {} to attach one",
                    domux_core::names::BIN_NAME
                ))
            })
    }

    /// The tab a `tab` param names (number, id or name in the view's workspace), or the
    /// view's current tab.
    pub fn resolve_tab_param(&self, tab: Option<&str>) -> Result<TabId, ApiError> {
        let client = self.view()?;
        let view = self
            .model
            .client(&client)
            .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
        match tab {
            Some(t) => self.model.resolve_tab(&view.workspace, t),
            None => Ok(view.tab.clone()),
        }
    }

    /// The pane a `pane` param names, or the view's focused pane.
    pub fn resolve_pane_param(&self, pane: Option<&str>) -> Result<PaneId, ApiError> {
        match pane {
            Some(p) => self.model.resolve_pane(p),
            None => {
                let client = self.view()?;
                self.model
                    .client_tab(&client)
                    .map(|t| t.focused.clone())
                    .ok_or_else(|| ApiError::not_found(format!("client {client} has no tab")))
            }
        }
    }

    /// The workpanel area of the smallest client on `tab`, or 80x24 when none shows it.
    /// The same rectangle `render::draw_panes` draws and `Core::sync_pane_sizes` sizes for.
    pub fn smallest_area(&self, tab: &TabId) -> Rect {
        crate::render::tab_workpanel(self.model, tab, UNVIEWED_SIZE)
    }

    /// A pane as the API reports it. `cols` and `rows` are its emulator's, which is the
    /// screen its program believes it has, so a pane with no runtime reports 0x0 rather
    /// than a size nothing is drawing.
    pub fn pane_info(&self, pane: &PaneId) -> Option<PaneInfo> {
        let p = self.model.pane(pane)?;
        let loc = self.model.pane_location(pane)?;
        let tab = self.model.tab(&loc.tab)?;
        let size = self
            .panes
            .get(pane)
            .map(|r| r.size())
            .unwrap_or(Size { cols: 0, rows: 0 });
        Some(PaneInfo {
            id: p.id.clone(),
            tab: loc.tab.clone(),
            cwd: p.cwd.clone(),
            command: p.command.clone(),
            title: p.title.clone(),
            pid: p.pid,
            focused: tab.focused == *pane,
            zoomed: tab.zoomed.as_ref() == Some(pane),
            copy_mode: p.copy_mode,
            cols: size.cols,
            rows: size.rows,
        })
    }
}

/// Every method, one arm each. No catch-all: a method added to the table in
/// `domux_core::api` fails to compile here until it has a handler.
///
/// A method declared before its handler answers `unavailable` from a stub arm at the end of
/// the match, and every such arm is on the register: `STILL_UNBUILT` in
/// `crate::core::tests`, which two tests there enforce from both directions. Building one
/// without deleting its line fails `only_the_expected_methods_still_answer_unavailable`, and
/// a stub added anywhere in this function - joined into an or-pattern or standing alone,
/// here or elsewhere - without a matching `STILL_UNBUILT` line fails
/// `every_unavailable_arm_in_dispatch_is_listed_in_still_unbuilt`. Both key off the words
/// "is not built yet", so a stub's message has to end in them.
///
/// The register is M2's (see `fed6573`, which removed the catch-all that used to answer for
/// a method with no handler, and `2d4d3a4`, which put those words back as a declared block):
/// Tasks 12 to 19 emptied it, which was the end state it was built to reach. M3 declares its
/// thirteen methods in one commit and fills them in over Tasks 10 to 18, so it is carrying
/// again, and it is what stops one of those methods from reaching the cut-over unbuilt and
/// unnoticed.
///
/// `workspace.resume` is the one refusal that is off the register on purpose: Task 19 gave it
/// a real handler, `api::workspace::resume`, which answers in words about agents rather than
/// in the register's, so neither direction looks at it.
pub fn dispatch(method: Method, ctx: &mut Ctx) -> Result<Value, ApiError> {
    use Method::*;
    match method {
        ServerInfo(_) => server::info(ctx),
        ServerStop(_) => server::stop(ctx),
        EventsSubscribe(_) => Err(ApiError::invalid_params(
            "events.subscribe only works on a control API connection, where it turns the connection into a stream",
        )),
        ConfigReload(_) => config::reload(ctx),
        ClientDetach(p) => client::detach(ctx, p),
        Help(p) => client::help(ctx, p),
        TabList(p) => tab::list(ctx, p),
        TabCreate(p) => tab::create(ctx, p),
        TabRename(p) => tab::rename(ctx, p),
        TabClearName(p) => tab::clear_name(ctx, p),
        TabClose(p) => tab::close(ctx, p),
        TabSelect(p) => tab::select(ctx, p),
        PaneList(p) => pane::list(ctx, p),
        PaneSplit(p) => pane::split(ctx, p),
        PaneClose(p) => pane::close(ctx, p),
        PaneFocus(p) => pane::focus(ctx, p),
        PaneZoom(p) => pane::zoom(ctx, p),
        PaneCopyMode(p) => pane::copy_mode(ctx, p),
        PaneResize(p) => pane::resize(ctx, p),
        PaneSendText(p) => pane::send_text(ctx, p),
        PaneSendKey(p) => pane::send_key(ctx, p),
        PaneRead(p) => pane::read(ctx, p),
        FocusLeft(p) => focus::step(ctx, p, Direction::Left),
        FocusRight(p) => focus::step(ctx, p, Direction::Right),
        FocusUp(p) => focus::step(ctx, p, Direction::Up),
        FocusDown(p) => focus::step(ctx, p, Direction::Down),
        FocusLast(p) => focus::last(ctx, p),
        FocusRegion(p) => focus::region(ctx, p),
        FocusPane(p) => focus::pane(ctx, p),
        SidebarToggle(p) => sidebar::toggle(ctx, p),
        SidebarShow(p) => sidebar::show(ctx, p),
        SidebarHide(p) => sidebar::hide(ctx, p),
        SwitcherOpen(p) => switcher::open(ctx, p),
        SwitcherClose(p) => switcher::close(ctx, p),
        ProjectList(p) => project::list(ctx, p),
        ProjectAdd(p) => project::add(ctx, p),
        ProjectRemove(p) => project::remove(ctx, p),
        WorkspaceCreate(p) => workspace::create(ctx, p),
        WorkspaceClear(p) => workspace::clear(ctx, p),
        WorkspaceDelete(p) => workspace::delete(ctx, p),
        WorkspaceList(p) => workspace::list(ctx, p),
        WorkspaceFocus(p) => workspace::focus(ctx, p),
        WorkspaceRename(p) => workspace::rename(ctx, p),
        WorkspaceClearName(p) => workspace::clear_name(ctx, p),
        WorkspaceResume(p) => workspace::resume(ctx, p),
        ListDown(p) => list::down(ctx, p),
        ListUp(p) => list::up(ctx, p),
        ListActivate(p) => list::activate(ctx, p),
        ListFilter(p) => list::filter(ctx, p),
        // M3's verbs are declared before their handlers, so every caller reads one shape of
        // this API from the first commit of the milestone. Each names what it is waiting for
        // and ends in the register's words, which is what puts it on `STILL_UNBUILT`: the
        // task that builds one replaces its arm and deletes its line there, and the register
        // fails if either half is forgotten.
        AgentList(_) => Err(ApiError::unavailable(
            "agent.list arrives with the agent records in M3 and is not built yet",
        )),
        AgentGet(_) => Err(ApiError::unavailable(
            "agent.get arrives with the agent records in M3 and is not built yet",
        )),
        AgentSelf(_) => Err(ApiError::unavailable(
            "agent.self arrives with the agent records in M3 and is not built yet",
        )),
        AgentReport(p) => agent::report(ctx, p),
        AgentFocus(_) => Err(ApiError::unavailable(
            "agent.focus arrives with the agents overlay in M3 and is not built yet",
        )),
        AgentDismiss(_) => Err(ApiError::unavailable(
            "agent.dismiss arrives with the agents overlay in M3 and is not built yet",
        )),
        AgentResume(_) => Err(ApiError::unavailable(
            "agent.resume arrives with resuming exited records in M3 and is not built yet",
        )),
        AgentSend(_) => Err(ApiError::unavailable(
            "agent.send arrives with messaging in M4 and is not built yet",
        )),
        AgentRead(_) => Err(ApiError::unavailable(
            "agent.read arrives with messaging in M4 and is not built yet",
        )),
        AgentWait(_) => Err(ApiError::unavailable(
            "agent.wait arrives with messaging in M4 and is not built yet",
        )),
        AgentsOpen(_) => Err(ApiError::unavailable(
            "agents.open arrives with the agents overlay in M3 and is not built yet",
        )),
        AgentsClose(_) => Err(ApiError::unavailable(
            "agents.close arrives with the agents overlay in M3 and is not built yet",
        )),
        FocusNextRegion(_) => Err(ApiError::unavailable(
            "focus.next_region arrives with the sidebar's Agents box in M3 and is not built yet",
        )),
    }
}

pub fn ok<T: serde::Serialize>(value: T) -> Result<Value, ApiError> {
    serde_json::to_value(value).map_err(|e| ApiError::internal(e.to_string()))
}
