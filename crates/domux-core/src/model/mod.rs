//! The Model: the root of all shared state, owned by the core task. Every mutation is a
//! method here that returns the events it produced. Nothing outside this crate writes fields.

pub mod focus;
pub mod layout;

pub use focus::{ConfirmKind, Focus, Overlay, PromptKind, RegionKind, TextInput};
pub use layout::{Direction, LayoutNode, Pane, PaneContent, Rect, SplitDir};

use crate::api::{ApiError, Event};
use crate::ids::{ClientId, IdGen, PaneId, ProjectId, TabId, WorkspaceId};
use crate::proto::Capabilities;
use domux_term::Size;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Model {
    /// M1: one implicit plain-folder project. M2: git projects.
    pub projects: Vec<Project>,
    /// Attached clients. Not persisted.
    #[serde(skip)]
    pub clients: Vec<ClientView>,
    pub last_workspace: Option<WorkspaceId>,
    #[serde(skip, default = "default_idgen")]
    idgen: IdGen,
    /// Counts client inputs so `most_recent_client` has an order. Not persisted.
    #[serde(skip)]
    activity_seq: u64,
    /// The ids of the most recently removed objects, oldest first. `id_exists` consults it
    /// alongside the live objects, so a closed pane's id is not handed straight back to a
    /// new pane while a client, an in-flight call or a queued message still holds the old
    /// one. Without it that stale id addresses a different pane and the keystroke or the
    /// close lands on the wrong one, which is worse than an honest `NotFound`.
    ///
    /// Bounded on purpose. `hex4` has 65536 values per prefix, so a set that only ever grew
    /// would in the end leave `next_id` nothing to draw and turn id reuse into a hang, a
    /// worse failure than the one this fixes. `RETIRED_CAPACITY` is where the oldest entry
    /// falls off.
    ///
    /// Deliberately **not** persisted to the state file, so task 10 must not serialise it.
    /// Every client re-fetches state after a restart and no id outlives that, so the window
    /// this closes is in-session only.
    #[serde(skip)]
    retired: VecDeque<String>,
}

/// Two models are equal when their content is equal: the projects, the attached clients and
/// `last_workspace`. `idgen`, `activity_seq` and `retired` are machinery rather than
/// content - the generator that mints ids, the counter that orders client activity, and the
/// record of ids not to hand out again. Two models holding the same objects are the same
/// model even though they will go on to mint different ids, so the machinery stays out of
/// the comparison.
///
/// The line is content against machinery. It is not observability: all three excluded
/// fields are observable through public items, since `next_id` reads `idgen` and `retired`,
/// `reseed` moves the first and `touch_client` moves `activity_seq`. And it is not
/// persistence: `clients` is `#[serde(skip)]` like the three excluded fields and is kept
/// anyway, because equality that ignored it would report two visibly different models as
/// the same. So a model saved and read back is genuinely not equal to the one that was
/// saved while a client is attached, and task 10 must compare what the state file holds -
/// the projects and `last_workspace`, or a model whose `clients` is empty - rather than
/// expect a bare `assert_eq!` round trip to hold.
impl PartialEq for Model {
    fn eq(&self, other: &Model) -> bool {
        // Destructured on purpose: a field added in a later milestone stops this compiling
        // until someone decides which side of the line it belongs on, rather than being
        // left out of equality with no error and no test failure.
        let Model {
            projects,
            clients,
            last_workspace,
            idgen: _,
            activity_seq: _,
            retired: _,
        } = self;
        *projects == other.projects
            && *clients == other.clients
            && *last_workspace == other.last_workspace
    }
}

fn default_idgen() -> IdGen {
    IdGen::from_seed(0x5eed)
}

/// How many removed ids `Model::retired` remembers.
///
/// The number has to do two things: cover the window in which something can still hold a
/// removed id - a client between events, an in-flight API call, a queued message - and stay
/// small against the 65536 values `hex4` produces, so `next_id` still finds a free one on
/// its first draw.
///
/// 1024 does both with room to spare. A stale reference lives for milliseconds, while
/// retiring 1024 objects takes a session's worth of opening and closing, so every id is
/// protected far longer than anything can hold it. And 1024 is 1.6% of the id space, so
/// even beside a thousand live objects the space stays about 97% free and the draw loop
/// almost never runs twice.
const RETIRED_CAPACITY: usize = 1024;

/// How many draws `next_id` gives the generator before it reports the id space full.
///
/// Draws are independent, so with a fraction `p` of a prefix's space occupied the chance of
/// this many collisions in a row is `p` to the 64th: about 1e-83 for a model holding a few
/// thousand objects, and still 5e-20 for a half-full space. Reaching the bound therefore
/// means the space is close to full rather than the draws being unlucky, which is what lets
/// the loop be bounded at all instead of spinning. It is not proof that every id is taken -
/// at 65000 in use the run still ends here 59% of the time - so the error `next_id` returns
/// reports the failed draws and does not count the ids in use.
const ID_DRAW_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectKind {
    Git { default_branch: String },
    Folder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root: PathBuf,
    pub kind: ProjectKind,
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceHandle {
    Main,
    Slot(u32),
}

impl fmt::Display for WorkspaceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceHandle::Main => f.write_str("main"),
            WorkspaceHandle::Slot(n) => write!(f, "workspace-{n}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub handle: WorkspaceHandle,
    pub name: Option<String>,
    pub path: PathBuf,
    pub tabs: Vec<Tab>,
    pub last_tab: Option<TabId>,
}

impl Workspace {
    /// The name when set, else the handle (architecture spec: a name replaces its handle).
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.handle.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tab {
    pub id: TabId,
    pub name: Option<String>,
    pub layout: LayoutNode,
    pub focused: PaneId,
    pub zoomed: Option<PaneId>,
    /// The pane focused before `focused`, for `focus.last`.
    #[serde(default)]
    pub last_focused: Option<PaneId>,
}

/// The sidebar's width in columns, borders included (roadmap section 5.5). M2 draws the
/// sidebar; M1 declares the constant so the name exists where the contract puts it.
pub const SIDEBAR_WIDTH: u16 = 38;

/// A leader chord in progress: the leader was pressed and the next key resolves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Chord {
    /// The leader as configured, for the indicator: `C-a`.
    pub leader: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClientView {
    pub id: ClientId,
    pub size: Size,
    pub caps: Capabilities,
    pub workspace: WorkspaceId,
    pub tab: TabId,
    pub focus: Focus,
    pub sidebar_open: bool,
    pub overlay: Option<Overlay>,
    pub chord: Option<Chord>,
    /// The filter text of a list overlay: the switcher in M2, the agents overlay in M3.
    /// `Overlay::Prompt` carries its own `TextInput`, so nothing in M1 reads this. It
    /// survives `select_tab` clearing `overlay`; whether a filter should persist between
    /// openings belongs to whoever builds those overlays.
    pub filter: String,
    /// The model's activity counter at this client's last input.
    #[serde(default)]
    pub last_active_seq: u64,
}

/// Where a pane lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLocation {
    pub project: ProjectId,
    pub workspace: WorkspaceId,
    pub tab: TabId,
}

/// Facts the server observed about a pane. `None` means "no observation", not "cleared".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneFacts {
    pub command: Option<String>,
    pub pid: Option<u32>,
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
}

impl Model {
    pub fn new(seed: u64) -> Model {
        Model {
            projects: Vec::new(),
            clients: Vec::new(),
            last_workspace: None,
            idgen: IdGen::from_seed(seed),
            activity_seq: 0,
            retired: VecDeque::new(),
        }
    }

    pub fn reseed(&mut self, seed: u64) {
        self.idgen = IdGen::from_seed(seed);
    }

    /// A fresh id with `prefix`, unique across every object in the model and against the
    /// recently removed ids in `retired`.
    ///
    /// Fails with an internal error when `ID_DRAW_LIMIT` draws all collide. There is
    /// nothing honest to return in that case: handing back a colliding id is the exact
    /// defect `retired` exists to prevent, and looping until one comes free would hang the
    /// core task instead. All the failure establishes is that every draw collided, which is
    /// what the message says: how much of the prefix's 65536-value space is taken is not
    /// something the draws measure.
    pub fn next_id(&mut self, prefix: &str) -> Result<String, ApiError> {
        for _ in 0..ID_DRAW_LIMIT {
            let id = format!("{prefix}_{}", self.idgen.hex4());
            if !self.id_exists(&id) {
                return Ok(id);
            }
        }
        // The message names the object the caller asked for, states what actually happened,
        // and gives an action that exists today. What happened is that every draw collided;
        // that the whole space is taken is an inference the draws do not support, since at
        // 65000 of 65536 ids in use a run of collisions this long still arrives more often
        // than not with hundreds of ids free. The prefix letter and the draw count stay out:
        // a reader cannot act on either, and the count would go stale the moment
        // `ID_DRAW_LIMIT` moved. Nothing removes a project or a workspace before M2, so
        // those two name the action that does free ids today, which is a restart: `retired`
        // is in-session only and starts empty.
        let message = match prefix {
            "pr" => "no free project id: every draw hit an id already in use or recently closed, so restart the server to clear the recently closed ids",
            "w" => "no free workspace id: every draw hit an id already in use or recently closed, so restart the server to clear the recently closed ids",
            "t" => "no free tab id: every draw hit an id already in use or recently closed, so close a tab",
            "p" => "no free pane id: every draw hit an id already in use or recently closed, so close a pane",
            "c" => "no free client id: every draw hit an id already in use or recently closed, so detach a client",
            // Nothing in this crate passes another prefix. An unknown one still gets a true
            // message rather than a guessed object name.
            _ => "no free id for that kind of object: every draw hit an id already in use or recently closed, so restart the server to clear the recently closed ids",
        };
        Err(ApiError::internal(message.to_string()))
    }

    /// Remembers a removed object's id so `next_id` will not reissue it while something may
    /// still hold it. The oldest entry falls off at `RETIRED_CAPACITY`.
    ///
    /// A plain queue with a linear membership test, not a queue plus a set: 1024 string
    /// comparisons cost nothing beside the live scan `id_exists` already does, and one
    /// structure cannot fall out of step with itself.
    fn retire(&mut self, id: String) {
        // `>=` in a loop rather than `==` once: production code cannot get above the bound,
        // but a test can construct such a state, and from one the invariant should still
        // hold after this returns rather than the queue growing without limit.
        while self.retired.len() >= RETIRED_CAPACITY {
            self.retired.pop_front();
        }
        self.retired.push_back(id);
    }

    fn id_exists(&self, id: &str) -> bool {
        if self.retired.iter().any(|r| r == id) {
            return true;
        }
        self.projects.iter().any(|p| {
            p.id.as_str() == id
                || p.workspaces.iter().any(|w| {
                    w.id.as_str() == id
                        || w.tabs.iter().any(|t| {
                            t.id.as_str() == id
                                || t.layout.panes().iter().any(|pn| pn.id.as_str() == id)
                        })
                })
        }) || self.clients.iter().any(|c| c.id.as_str() == id)
    }

    /// Registers a plain folder as a project with its `main` workspace and no tabs. The
    /// name is the folder's last path component. Returns no events in M1 (`project.added`
    /// is an M2 event), and fails only when the id space is full.
    pub fn add_folder_project(
        &mut self,
        root: PathBuf,
    ) -> Result<(ProjectId, WorkspaceId, Vec<Event>), ApiError> {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let pid = ProjectId(self.next_id("pr")?);
        let wid = WorkspaceId(self.next_id("w")?);
        self.projects.push(Project {
            id: pid.clone(),
            name,
            root: root.clone(),
            kind: ProjectKind::Folder,
            workspaces: vec![Workspace {
                id: wid.clone(),
                handle: WorkspaceHandle::Main,
                name: None,
                path: root,
                tabs: Vec::new(),
                last_tab: None,
            }],
        });
        if self.last_workspace.is_none() {
            self.last_workspace = Some(wid.clone());
        }
        Ok((pid, wid, Vec::new()))
    }

    pub fn first_workspace(&self) -> Option<WorkspaceId> {
        self.projects
            .first()
            .and_then(|p| p.workspaces.first())
            .map(|w| w.id.clone())
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Option<&Workspace> {
        self.projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .find(|w| &w.id == id)
    }

    pub fn workspace_mut(&mut self, id: &WorkspaceId) -> Option<&mut Workspace> {
        self.projects
            .iter_mut()
            .flat_map(|p| p.workspaces.iter_mut())
            .find(|w| &w.id == id)
    }

    pub fn project_of_workspace(&self, id: &WorkspaceId) -> Option<&Project> {
        self.projects
            .iter()
            .find(|p| p.workspaces.iter().any(|w| &w.id == id))
    }

    pub fn tab(&self, id: &TabId) -> Option<&Tab> {
        self.projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .flat_map(|w| w.tabs.iter())
            .find(|t| &t.id == id)
    }

    pub fn tab_mut(&mut self, id: &TabId) -> Option<&mut Tab> {
        self.projects
            .iter_mut()
            .flat_map(|p| p.workspaces.iter_mut())
            .flat_map(|w| w.tabs.iter_mut())
            .find(|t| &t.id == id)
    }

    pub fn workspace_of_tab(&self, id: &TabId) -> Option<&Workspace> {
        self.projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .find(|w| w.tabs.iter().any(|t| &t.id == id))
    }

    pub fn pane_location(&self, id: &PaneId) -> Option<PaneLocation> {
        for p in &self.projects {
            for w in &p.workspaces {
                for t in &w.tabs {
                    if t.layout.contains(id) {
                        return Some(PaneLocation {
                            project: p.id.clone(),
                            workspace: w.id.clone(),
                            tab: t.id.clone(),
                        });
                    }
                }
            }
        }
        None
    }

    pub fn pane(&self, id: &PaneId) -> Option<&Pane> {
        self.projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .flat_map(|w| w.tabs.iter())
            .find_map(|t| t.layout.pane(id))
    }

    pub fn pane_mut(&mut self, id: &PaneId) -> Option<&mut Pane> {
        self.projects
            .iter_mut()
            .flat_map(|p| p.workspaces.iter_mut())
            .flat_map(|w| w.tabs.iter_mut())
            .find_map(|t| t.layout.pane_mut(id))
    }

    pub fn all_pane_ids(&self) -> Vec<PaneId> {
        self.projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .flat_map(|w| w.tabs.iter())
            .flat_map(|t| t.layout.pane_ids())
            .collect()
    }

    pub fn client(&self, id: &ClientId) -> Option<&ClientView> {
        self.clients.iter().find(|c| &c.id == id)
    }

    pub fn client_mut(&mut self, id: &ClientId) -> Option<&mut ClientView> {
        self.clients.iter_mut().find(|c| &c.id == id)
    }

    pub fn client_tab(&self, id: &ClientId) -> Option<&Tab> {
        let c = self.client(id)?;
        self.tab(&c.tab)
    }

    /// The client with the latest input, for API calls that omit `client`.
    pub fn most_recent_client(&self) -> Option<ClientId> {
        self.clients
            .iter()
            .max_by_key(|c| c.last_active_seq)
            .map(|c| c.id.clone())
    }

    pub fn touch_client(&mut self, id: &ClientId) {
        self.activity_seq += 1;
        let seq = self.activity_seq;
        if let Some(c) = self.client_mut(id) {
            c.last_active_seq = seq;
        }
    }

    pub fn attach_client(&mut self, view: ClientView) -> Vec<Event> {
        let id = view.id.clone();
        self.clients.push(view);
        self.touch_client(&id);
        vec![Event::ClientAttached { client: id }]
    }

    pub fn detach_client(&mut self, id: &ClientId) -> Vec<Event> {
        let before = self.clients.len();
        self.clients.retain(|c| &c.id != id);
        if self.clients.len() == before {
            Vec::new()
        } else {
            self.retire(id.to_string());
            vec![Event::ClientDetached { client: id.clone() }]
        }
    }

    /// Adds a tab with one pane whose shell starts in `cwd`. The tab becomes the
    /// workspace's last tab; clients are not moved (the caller selects it if it wants).
    pub fn create_tab(
        &mut self,
        ws: &WorkspaceId,
        cwd: PathBuf,
    ) -> Result<(TabId, PaneId, Vec<Event>), ApiError> {
        let tid = TabId(self.next_id("t")?);
        let pid = PaneId(self.next_id("p")?);
        let w = self
            .workspace_mut(ws)
            .ok_or_else(|| ApiError::not_found(format!("workspace {ws} does not exist")))?;
        let pane = Pane {
            id: pid.clone(),
            cwd: cwd.clone(),
            command: None,
            title: None,
            pid: None,
            copy_mode: false,
        };
        w.tabs.push(Tab {
            id: tid.clone(),
            name: None,
            layout: LayoutNode::leaf(pane),
            focused: pid.clone(),
            zoomed: None,
            last_focused: None,
        });
        w.last_tab = Some(tid.clone());
        Ok((
            tid.clone(),
            pid.clone(),
            vec![
                Event::TabCreated {
                    workspace: ws.clone(),
                    tab: tid.clone(),
                },
                Event::PaneSpawned {
                    tab: tid,
                    pane: pid,
                    cwd,
                },
            ],
        ))
    }

    /// Sets or clears the name. `Some("")` and whitespace clear (roadmap 5.7).
    pub fn rename_tab(
        &mut self,
        tab: &TabId,
        name: Option<String>,
    ) -> Result<Vec<Event>, ApiError> {
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        let t = self.tab_mut(tab).ok_or_else(|| {
            ApiError::not_found(format!("tab {tab} does not exist; run domux2 api tab.list"))
        })?;
        t.name = name.clone();
        Ok(vec![Event::TabRenamed {
            tab: tab.clone(),
            name,
        }])
    }

    /// Removes a tab and returns the panes the server must kill. Clients on the tab move to
    /// the neighbour that took its place, and each move is reported as `tab.selected`, the
    /// one event that says a client changed tab. The last tab of a workspace can be closed;
    /// the caller then creates a fresh tab so a workspace never has none.
    pub fn close_tab(&mut self, tab: &TabId) -> Result<(Vec<PaneId>, Vec<Event>), ApiError> {
        let ws_id = self
            .workspace_of_tab(tab)
            .map(|w| w.id.clone())
            .ok_or_else(|| {
                ApiError::not_found(format!("tab {tab} does not exist; run domux2 api tab.list"))
            })?;
        let w = self.workspace_mut(&ws_id).expect("workspace exists");
        let index = w
            .tabs
            .iter()
            .position(|t| &t.id == tab)
            .expect("tab in workspace");
        let removed = w.tabs.remove(index);
        let panes = removed.layout.pane_ids();
        let replacement = w
            .tabs
            .get(index)
            .or_else(|| w.tabs.last())
            .map(|t| (t.id.clone(), t.focused.clone()));
        if w.last_tab.as_ref() == Some(tab) {
            w.last_tab = replacement.as_ref().map(|(t, _)| t.clone());
        }
        let mut events = Vec::new();
        for p in &panes {
            events.push(Event::PaneClosed {
                tab: tab.clone(),
                pane: p.clone(),
            });
        }
        events.push(Event::TabClosed {
            workspace: ws_id,
            tab: tab.clone(),
        });
        for c in &mut self.clients {
            if &c.tab == tab {
                if let Some((t, p)) = &replacement {
                    c.tab = t.clone();
                    c.focus = Focus::Pane(p.clone());
                    events.push(Event::TabSelected {
                        client: c.id.clone(),
                        tab: t.clone(),
                    });
                }
            }
        }
        self.retire(tab.to_string());
        for p in &panes {
            self.retire(p.to_string());
        }
        Ok((panes, events))
    }

    /// Moves one client to a tab in its workspace and focuses that tab's focused pane.
    pub fn select_tab(&mut self, client: &ClientId, tab: &TabId) -> Result<Vec<Event>, ApiError> {
        let focused = self.tab(tab).map(|t| t.focused.clone()).ok_or_else(|| {
            ApiError::not_found(format!("tab {tab} does not exist; run domux2 api tab.list"))
        })?;
        let ws_id = self
            .workspace_of_tab(tab)
            .map(|w| w.id.clone())
            .expect("tab has a workspace");
        let c = self.client_mut(client).ok_or_else(|| {
            ApiError::not_found(format!(
                "client {client} is not attached; run domux2 api server.info"
            ))
        })?;
        c.tab = tab.clone();
        c.workspace = ws_id.clone();
        c.focus = Focus::Pane(focused);
        c.overlay = None;
        if let Some(w) = self.workspace_mut(&ws_id) {
            w.last_tab = Some(tab.clone());
        }
        Ok(vec![Event::TabSelected {
            client: client.clone(),
            tab: tab.clone(),
        }])
    }

    /// Splits `pane` and focuses the new pane. A zoom on the tab is cleared, and reported:
    /// `pane.zoomed` is the only event that says a zoom changed, so a client that tracks
    /// zoom from the stream would otherwise keep drawing the old pane full screen.
    pub fn split_pane(
        &mut self,
        pane: &PaneId,
        dir: Direction,
        cwd: PathBuf,
    ) -> Result<(PaneId, Vec<Event>), ApiError> {
        let loc = self.pane_location(pane).ok_or_else(|| {
            ApiError::not_found(format!(
                "pane {pane} does not exist; run domux2 api pane.list"
            ))
        })?;
        let new_id = PaneId(self.next_id("p")?);
        let new = Pane {
            id: new_id.clone(),
            cwd: cwd.clone(),
            command: None,
            title: None,
            pid: None,
            copy_mode: false,
        };
        let t = self.tab_mut(&loc.tab).expect("tab exists");
        // `pane_location` already found `pane` in this tab's layout, so the split cannot
        // miss. If it ever did, the events below would announce a pane the layout does not
        // hold and `focused` would name it: silent corruption, caught here instead.
        let split = t.layout.split_leaf(pane, dir, new);
        debug_assert!(split, "split_leaf missed {pane}, which pane_location found");
        let cleared_zoom = t.zoomed.take().is_some();
        t.last_focused = Some(t.focused.clone());
        t.focused = new_id.clone();
        let mut events = vec![Event::PaneSpawned {
            tab: loc.tab.clone(),
            pane: new_id.clone(),
            cwd,
        }];
        if cleared_zoom {
            events.push(Event::PaneZoomed {
                tab: loc.tab.clone(),
                pane: None,
            });
        }
        events.push(Event::PaneFocused {
            tab: loc.tab.clone(),
            pane: new_id.clone(),
        });
        let tab_id = loc.tab.clone();
        for c in &mut self.clients {
            if c.tab == tab_id && matches!(c.focus, Focus::Pane(_)) {
                c.focus = Focus::Pane(new_id.clone());
            }
        }
        Ok((new_id, events))
    }

    /// Closes a pane. When it was the tab's last pane the tab closes too, and the returned
    /// `Option<TabId>` names it.
    ///
    /// Focus moves only when the closed pane held it, and then in two steps: to
    /// `last_focused` when that pane is still in the layout, else to whichever pane now
    /// holds the closed one's position in reading order.
    ///
    /// The three parts of the tuple are the caller's whole job after a close: the panes to
    /// kill, the tab that went with them, and the events to publish. Naming it would hide
    /// that from the call site, so the tuple stays and the lint is allowed here.
    #[allow(clippy::type_complexity)]
    pub fn close_pane(
        &mut self,
        pane: &PaneId,
    ) -> Result<(Vec<PaneId>, Option<TabId>, Vec<Event>), ApiError> {
        let loc = self.pane_location(pane).ok_or_else(|| {
            ApiError::not_found(format!(
                "pane {pane} does not exist; run domux2 api pane.list"
            ))
        })?;
        let is_last = self
            .tab(&loc.tab)
            .map(|t| t.layout.pane_ids().len() == 1)
            .unwrap_or(false);
        if is_last {
            let (panes, events) = self.close_tab(&loc.tab)?;
            return Ok((panes, Some(loc.tab), events));
        }
        let t = self.tab_mut(&loc.tab).expect("tab exists");
        let before = t.layout.pane_ids();
        let index = before.iter().position(|p| p == pane).unwrap_or(0);
        // Unreachable: `pane_location` found the pane and `is_last` ruled out the
        // single-leaf case, the only two ways `remove_leaf` returns `None`. If it ever did,
        // `pane.closed` would name a pane still in the layout and focus would be set right
        // back to it.
        let removed = t.layout.remove_leaf(pane);
        debug_assert!(
            removed.is_some(),
            "remove_leaf missed {pane}, which pane_location found"
        );
        let after = t.layout.pane_ids();
        let mut events = vec![Event::PaneClosed {
            tab: loc.tab.clone(),
            pane: pane.clone(),
        }];
        if t.zoomed.as_ref() == Some(pane) {
            t.zoomed = None;
            events.push(Event::PaneZoomed {
                tab: loc.tab.clone(),
                pane: None,
            });
        }
        if t.last_focused.as_ref() == Some(pane) {
            t.last_focused = None;
        }
        if &t.focused == pane {
            let next = t
                .last_focused
                .clone()
                .filter(|p| after.contains(p))
                .unwrap_or_else(|| {
                    after
                        .get(index.min(after.len() - 1))
                        .cloned()
                        .expect("a pane remains")
                });
            t.focused = next.clone();
            t.last_focused = None;
            events.push(Event::PaneFocused {
                tab: loc.tab.clone(),
                pane: next.clone(),
            });
            let tab_id = loc.tab.clone();
            for c in &mut self.clients {
                if c.tab == tab_id && c.focus == Focus::Pane(pane.clone()) {
                    c.focus = Focus::Pane(next.clone());
                }
            }
        }
        self.retire(pane.to_string());
        Ok((vec![pane.clone()], None, events))
    }

    /// Focuses `pane` in its tab. Every client on that tab whose focus is a pane follows.
    pub fn focus_pane(&mut self, pane: &PaneId) -> Result<Vec<Event>, ApiError> {
        let loc = self.pane_location(pane).ok_or_else(|| {
            ApiError::not_found(format!(
                "pane {pane} does not exist; run domux2 api pane.list"
            ))
        })?;
        let t = self.tab_mut(&loc.tab).expect("tab exists");
        if &t.focused == pane {
            return Ok(Vec::new());
        }
        t.last_focused = Some(t.focused.clone());
        t.focused = pane.clone();
        let tab_id = loc.tab.clone();
        for c in &mut self.clients {
            if c.tab == tab_id && matches!(c.focus, Focus::Pane(_)) {
                c.focus = Focus::Pane(pane.clone());
            }
        }
        Ok(vec![Event::PaneFocused {
            tab: loc.tab,
            pane: pane.clone(),
        }])
    }

    pub fn toggle_zoom(&mut self, tab: &TabId) -> Result<Vec<Event>, ApiError> {
        let t = self.tab_mut(tab).ok_or_else(|| {
            ApiError::not_found(format!("tab {tab} does not exist; run domux2 api tab.list"))
        })?;
        t.zoomed = if t.zoomed.is_some() {
            None
        } else {
            Some(t.focused.clone())
        };
        Ok(vec![Event::PaneZoomed {
            tab: tab.clone(),
            pane: t.zoomed.clone(),
        }])
    }

    /// Resizes through the layout tree. Returns whether a split with the requested axis
    /// owned the request, which is not the same as the geometry having moved: the ratio may
    /// have been pinned at the `MIN_BOX` floor already. Do not read this as "something
    /// changed".
    pub fn resize_pane(
        &mut self,
        pane: &PaneId,
        dir: Direction,
        cells: u16,
        area: Rect,
    ) -> Result<bool, ApiError> {
        let loc = self.pane_location(pane).ok_or_else(|| {
            ApiError::not_found(format!(
                "pane {pane} does not exist; run domux2 api pane.list"
            ))
        })?;
        let t = self.tab_mut(&loc.tab).expect("tab exists");
        Ok(t.layout.resize(pane, dir, cells, area))
    }

    /// Records observed facts. Each `Some` overwrites; each `None` leaves the field alone.
    pub fn set_pane_facts(&mut self, pane: &PaneId, facts: PaneFacts) {
        if let Some(p) = self.pane_mut(pane) {
            if facts.command.is_some() {
                p.command = facts.command;
            }
            if facts.pid.is_some() {
                p.pid = facts.pid;
            }
            if let Some(cwd) = facts.cwd {
                p.cwd = cwd;
            }
            if facts.title.is_some() {
                p.title = facts.title;
            }
        }
    }

    pub fn set_pane_copy_mode(&mut self, pane: &PaneId, on: bool) {
        if let Some(p) = self.pane_mut(pane) {
            p.copy_mode = on;
        }
    }

    /// A tab in `ws` by 1-based number, by id, or by name.
    ///
    /// A target that parses as a number is a position and nothing else, so a tab named `2`
    /// cannot be reached by that name; the number branch answers first. Ids and names share
    /// the second branch, ids first. The two misses word differently on purpose: an
    /// out-of-range number says how many tabs the workspace has, while an id or name that
    /// matches nothing quotes the target back.
    pub fn resolve_tab(&self, ws: &WorkspaceId, target: &str) -> Result<TabId, ApiError> {
        let w = self
            .workspace(ws)
            .ok_or_else(|| ApiError::not_found(format!("workspace {ws} does not exist")))?;
        if let Ok(n) = target.parse::<usize>() {
            return w
                .tabs
                .get(n.wrapping_sub(1))
                .map(|t| t.id.clone())
                .ok_or_else(|| {
                    ApiError::not_found(format!(
                        "tab {n} does not exist; this workspace has {} tabs",
                        w.tabs.len()
                    ))
                });
        }
        w.tabs
            .iter()
            .find(|t| t.id.as_str() == target || t.name.as_deref() == Some(target))
            .map(|t| t.id.clone())
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "tab {target:?} does not exist; run domux2 api tab.list"
                ))
            })
    }

    pub fn resolve_pane(&self, target: &str) -> Result<PaneId, ApiError> {
        let id: PaneId = target.parse().map_err(|_| {
            ApiError::invalid_params(format!(
                "{target:?} is not a pane id; pane ids look like p_8f2a"
            ))
        })?;
        if self.pane(&id).is_some() {
            Ok(id)
        } else {
            Err(ApiError::not_found(format!(
                "pane {target} does not exist; run domux2 api pane.list"
            )))
        }
    }

    pub fn root_of(&self, path: &Path) -> Option<&Project> {
        self.projects.iter().find(|p| p.root == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Event;
    use std::path::PathBuf;

    fn model_with_one_tab() -> (Model, WorkspaceId, TabId, PaneId) {
        let mut m = Model::new(7);
        let (_, ws, _) = m
            .add_folder_project(PathBuf::from("/Users/pranav/projects/domux"))
            .unwrap();
        let (tab, pane, _) = m
            .create_tab(&ws, PathBuf::from("/Users/pranav/projects/domux"))
            .unwrap();
        (m, ws, tab, pane)
    }

    /// One tab with panes `a`, `b`, `c` in reading order. `focused` is `c` and
    /// `last_focused` is `b`, which two panes cannot produce: the focus rules only differ
    /// once a third pane exists.
    fn three_panes() -> (Model, WorkspaceId, TabId, PaneId, PaneId, PaneId) {
        let (mut m, ws, tab, a) = model_with_one_tab();
        let (b, _) = m
            .split_pane(&a, Direction::Right, PathBuf::from("/x"))
            .unwrap();
        let (c, _) = m
            .split_pane(&b, Direction::Right, PathBuf::from("/x"))
            .unwrap();
        assert_eq!(
            m.tab(&tab).unwrap().layout.pane_ids(),
            vec![a.clone(), b.clone(), c.clone()]
        );
        (m, ws, tab, a, b, c)
    }

    fn client(id: &str, ws: &WorkspaceId, tab: &TabId, pane: &PaneId) -> ClientView {
        ClientView {
            id: ClientId(id.to_string()),
            size: Size { cols: 80, rows: 24 },
            caps: Capabilities::default(),
            workspace: ws.clone(),
            tab: tab.clone(),
            focus: Focus::Pane(pane.clone()),
            sidebar_open: false,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
        }
    }

    #[test]
    fn folder_project_has_a_main_workspace_named_after_the_folder() {
        let mut m = Model::new(7);
        let (pid, ws, events) = m
            .add_folder_project(PathBuf::from("/Users/pranav/projects/domux"))
            .unwrap();
        let p = &m.projects[0];
        assert_eq!(p.id, pid);
        assert_eq!(p.name, "domux");
        assert_eq!(p.kind, ProjectKind::Folder);
        assert_eq!(p.workspaces[0].id, ws);
        assert_eq!(p.workspaces[0].handle, WorkspaceHandle::Main);
        assert_eq!(p.workspaces[0].handle.to_string(), "main");
        assert!(events.is_empty(), "project.added is an M2 event");
    }

    #[test]
    fn create_tab_adds_one_tab_with_one_pane_and_reports_both() {
        let (m, ws, tab, pane) = model_with_one_tab();
        let w = m.workspace(&ws).unwrap();
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.tabs[0].id, tab);
        assert_eq!(w.tabs[0].focused, pane);
        assert_eq!(w.tabs[0].name, None);
        assert_eq!(w.last_tab, Some(tab.clone()));
        let mut m2 = Model::new(7);
        let (_, ws2, _) = m2.add_folder_project(PathBuf::from("/x")).unwrap();
        let (t, p, events) = m2.create_tab(&ws2, PathBuf::from("/x")).unwrap();
        assert_eq!(
            events,
            vec![
                Event::TabCreated {
                    workspace: ws2.clone(),
                    tab: t.clone()
                },
                Event::PaneSpawned {
                    tab: t,
                    pane: p,
                    cwd: PathBuf::from("/x")
                }
            ]
        );
    }

    #[test]
    fn rename_tab_with_empty_name_clears_it() {
        let (mut m, _, tab, _) = model_with_one_tab();
        let events = m.rename_tab(&tab, Some("tests".into())).unwrap();
        assert_eq!(m.tab(&tab).unwrap().name.as_deref(), Some("tests"));
        assert_eq!(
            events,
            vec![Event::TabRenamed {
                tab: tab.clone(),
                name: Some("tests".into())
            }]
        );
        m.rename_tab(&tab, Some("".into())).unwrap();
        assert_eq!(m.tab(&tab).unwrap().name, None);
        m.rename_tab(&tab, Some("   ".into())).unwrap();
        assert_eq!(m.tab(&tab).unwrap().name, None, "whitespace is empty");
    }

    #[test]
    fn split_pane_focuses_the_new_pane_and_close_pane_refocuses_a_neighbour() {
        let (mut m, _, tab, pane) = model_with_one_tab();
        let (new, events) = m
            .split_pane(&pane, Direction::Right, PathBuf::from("/tmp"))
            .unwrap();
        assert_eq!(m.tab(&tab).unwrap().focused, new);
        assert_eq!(m.tab(&tab).unwrap().last_focused, Some(pane.clone()));
        assert_eq!(
            events[0],
            Event::PaneSpawned {
                tab: tab.clone(),
                pane: new.clone(),
                cwd: PathBuf::from("/tmp")
            }
        );
        assert_eq!(
            events[1],
            Event::PaneFocused {
                tab: tab.clone(),
                pane: new.clone()
            }
        );
        let (closed, closed_tab, events) = m.close_pane(&new).unwrap();
        assert_eq!(closed, vec![new.clone()]);
        assert_eq!(closed_tab, None);
        assert_eq!(m.tab(&tab).unwrap().focused, pane);
        assert!(events.contains(&Event::PaneClosed {
            tab: tab.clone(),
            pane: new
        }));
    }

    #[test]
    fn closing_the_last_pane_closes_the_tab() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        let (t2, _, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let (closed, closed_tab, events) = m.close_pane(&pane).unwrap();
        assert_eq!(closed, vec![pane]);
        assert_eq!(closed_tab, Some(tab.clone()));
        assert!(events.contains(&Event::TabClosed {
            workspace: ws.clone(),
            tab
        }));
        assert_eq!(m.workspace(&ws).unwrap().tabs.len(), 1);
        assert_eq!(m.workspace(&ws).unwrap().last_tab, Some(t2));
    }

    #[test]
    fn zoom_toggles_and_clears_when_the_zoomed_pane_closes() {
        let (mut m, _, tab, pane) = model_with_one_tab();
        let (new, _) = m
            .split_pane(&pane, Direction::Down, PathBuf::from("/tmp"))
            .unwrap();
        let events = m.toggle_zoom(&tab).unwrap();
        assert_eq!(m.tab(&tab).unwrap().zoomed, Some(new.clone()));
        assert_eq!(
            events,
            vec![Event::PaneZoomed {
                tab: tab.clone(),
                pane: Some(new.clone())
            }]
        );
        m.toggle_zoom(&tab).unwrap();
        assert_eq!(m.tab(&tab).unwrap().zoomed, None);
        m.toggle_zoom(&tab).unwrap();
        m.close_pane(&new).unwrap();
        assert_eq!(m.tab(&tab).unwrap().zoomed, None);
    }

    #[test]
    fn select_tab_moves_only_that_client_and_records_last_tab() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        let (t2, p2, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.attach_client(client("c_0001", &ws, &tab, &pane));
        m.attach_client(client("c_0002", &ws, &tab, &pane));
        let events = m.select_tab(&ClientId("c_0001".into()), &t2).unwrap();
        assert_eq!(m.client(&ClientId("c_0001".into())).unwrap().tab, t2);
        assert_eq!(
            m.client(&ClientId("c_0001".into())).unwrap().focus,
            Focus::Pane(p2)
        );
        assert_eq!(m.client(&ClientId("c_0002".into())).unwrap().tab, tab);
        assert_eq!(m.workspace(&ws).unwrap().last_tab, Some(t2.clone()));
        assert_eq!(
            events,
            vec![Event::TabSelected {
                client: ClientId("c_0001".into()),
                tab: t2
            }]
        );
    }

    #[test]
    fn clients_on_a_closed_tab_move_to_the_nearest_tab() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        let (t2, _, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.attach_client(client("c_0001", &ws, &t2, &pane));
        m.close_tab(&t2).unwrap();
        assert_eq!(m.client(&ClientId("c_0001".into())).unwrap().tab, tab);
    }

    #[test]
    fn resolve_tab_accepts_number_or_id() {
        let (mut m, ws, tab, _) = model_with_one_tab();
        let (t2, _, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        assert_eq!(m.resolve_tab(&ws, "1").unwrap(), tab);
        assert_eq!(m.resolve_tab(&ws, "2").unwrap(), t2);
        assert_eq!(m.resolve_tab(&ws, t2.as_str()).unwrap(), t2);
        let err = m.resolve_tab(&ws, "9").unwrap_err();
        assert_eq!(err.code, crate::api::ErrorCode::NotFound);
        assert_eq!(
            err.message,
            "tab 9 does not exist; this workspace has 2 tabs"
        );
    }

    #[test]
    fn split_pane_reports_the_zoom_it_clears() {
        let (mut m, _, tab, pane) = model_with_one_tab();
        let (second, _) = m
            .split_pane(&pane, Direction::Right, PathBuf::from("/tmp"))
            .unwrap();
        m.toggle_zoom(&tab).unwrap();
        assert_eq!(m.tab(&tab).unwrap().zoomed, Some(second.clone()));

        let (third, events) = m
            .split_pane(&second, Direction::Down, PathBuf::from("/tmp"))
            .unwrap();
        assert_eq!(m.tab(&tab).unwrap().zoomed, None);
        assert_eq!(
            events,
            vec![
                Event::PaneSpawned {
                    tab: tab.clone(),
                    pane: third.clone(),
                    cwd: PathBuf::from("/tmp")
                },
                Event::PaneZoomed {
                    tab: tab.clone(),
                    pane: None
                },
                Event::PaneFocused {
                    tab: tab.clone(),
                    pane: third.clone()
                },
            ],
            "a client that tracks zoom from the stream has to be told the zoom went"
        );

        let (fourth, events) = m
            .split_pane(&third, Direction::Down, PathBuf::from("/tmp"))
            .unwrap();
        assert_eq!(
            events,
            vec![
                Event::PaneSpawned {
                    tab: tab.clone(),
                    pane: fourth.clone(),
                    cwd: PathBuf::from("/tmp")
                },
                Event::PaneFocused {
                    tab: tab.clone(),
                    pane: fourth
                },
            ],
            "no zoom was set, so there is nothing to report"
        );
    }

    #[test]
    fn close_tab_reports_the_tab_each_moved_client_lands_on() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        let (t2, p2, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.attach_client(client("c_0001", &ws, &t2, &p2));
        m.attach_client(client("c_0002", &ws, &t2, &p2));
        m.attach_client(client("c_0003", &ws, &tab, &pane));
        let (_, events) = m.close_tab(&t2).unwrap();
        assert_eq!(
            events,
            vec![
                Event::PaneClosed {
                    tab: t2.clone(),
                    pane: p2
                },
                Event::TabClosed {
                    workspace: ws.clone(),
                    tab: t2.clone()
                },
                Event::TabSelected {
                    client: ClientId("c_0001".into()),
                    tab: tab.clone()
                },
                Event::TabSelected {
                    client: ClientId("c_0002".into()),
                    tab: tab.clone()
                },
            ],
            "every client the close moved is named, and only those"
        );
        assert_eq!(m.client(&ClientId("c_0003".into())).unwrap().tab, tab);
    }

    #[test]
    fn ids_are_not_reissued_after_a_tab_is_removed() {
        // Seed 1 repeats a `hex4` value within 77 draws, so a live-only uniqueness check
        // hands a closed tab's id to a new tab well inside this loop.
        let mut m = Model::new(1);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..300 {
            let (t, p, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
            assert!(seen.insert(t.0.clone()), "tab id {t} was reissued");
            assert!(seen.insert(p.0.clone()), "pane id {p} was reissued");
            m.close_tab(&t).unwrap();
        }
    }

    #[test]
    fn ids_are_not_reissued_after_a_pane_is_removed() {
        // Closing a pane is the removal the product performs most, and it retires on its
        // own path rather than through `close_tab`'s loop. Splitting one surviving pane
        // and closing the new one keeps the tab alive, so only the pane path runs here.
        let mut m = Model::new(1);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let (_, first, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let mut seen = std::collections::HashSet::new();
        seen.insert(first.0.clone());
        for _ in 0..300 {
            let (p, _) = m
                .split_pane(&first, Direction::Right, PathBuf::from("/x"))
                .unwrap();
            assert!(seen.insert(p.0.clone()), "pane id {p} was reissued");
            m.close_pane(&p).unwrap();
        }
    }

    #[test]
    fn ids_are_not_reissued_after_a_client_detaches() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        let cid = ClientId(m.next_id("c").unwrap());
        m.attach_client(ClientView {
            id: cid.clone(),
            ..client("c_0001", &ws, &tab, &pane)
        });
        m.detach_client(&cid);
        // Rewind the generator to the stream that already produced `cid`, so the next few
        // draws offer that exact value again.
        m.reseed(7);
        let redrawn: Vec<String> = (0..8).map(|_| m.next_id("c").unwrap()).collect();
        assert!(
            !redrawn.contains(&cid.0),
            "a detached client's id {cid} came back as {redrawn:?}"
        );
    }

    #[test]
    fn close_pane_prefers_last_focused_over_the_positional_neighbour() {
        let (mut m, _, tab, a, b, c) = three_panes();
        // Focusing `a` records `c` as the pane focused before it.
        m.focus_pane(&a).unwrap();
        assert_eq!(m.tab(&tab).unwrap().focused, a);
        assert_eq!(m.tab(&tab).unwrap().last_focused, Some(c.clone()));

        // `a` sits at index 0, so the positional fallback would answer `b`. The two rules
        // disagree here, which is the only way to tell them apart.
        let (closed, closed_tab, events) = m.close_pane(&a).unwrap();
        assert_eq!(closed, vec![a.clone()]);
        assert_eq!(closed_tab, None);
        assert_eq!(
            m.tab(&tab).unwrap().focused,
            c,
            "last_focused wins while that pane survives"
        );
        assert_eq!(
            events,
            vec![
                Event::PaneClosed {
                    tab: tab.clone(),
                    pane: a
                },
                Event::PaneFocused {
                    tab: tab.clone(),
                    pane: c.clone()
                },
            ],
            "the whole event vector, not just its first element"
        );

        // The close cleared last_focused, so the fallback answers this time: `c` is at
        // index 1 of [b, c] and only `b` remains.
        m.close_pane(&c).unwrap();
        assert_eq!(m.tab(&tab).unwrap().focused, b);
    }

    #[test]
    fn close_pane_clears_a_last_focused_that_names_the_closed_pane() {
        let (mut m, _, tab, _a, b, c) = three_panes();
        assert_eq!(m.tab(&tab).unwrap().last_focused, Some(b.clone()));

        // `b` is not the focused pane, so the refocus branch never runs and this clear is
        // the only thing that can stop last_focused naming a pane that is gone.
        m.close_pane(&b).unwrap();
        assert_eq!(m.tab(&tab).unwrap().focused, c, "focus did not move");
        assert_eq!(m.tab(&tab).unwrap().last_focused, None);
    }

    #[test]
    fn close_tab_moves_last_tab_off_the_tab_it_removes() {
        let (mut m, ws, t1, _) = model_with_one_tab();
        let (t2, _, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let (t3, _, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        assert_eq!(m.workspace(&ws).unwrap().last_tab, Some(t3.clone()));

        m.close_tab(&t1).unwrap();
        assert_eq!(
            m.workspace(&ws).unwrap().last_tab,
            Some(t3.clone()),
            "closing another tab leaves last_tab alone"
        );

        m.close_tab(&t3).unwrap();
        assert_eq!(
            m.workspace(&ws).unwrap().last_tab,
            Some(t2),
            "or last_tab names a tab that no longer exists"
        );
    }

    #[test]
    fn split_pane_moves_only_the_pane_focused_clients_on_that_tab() {
        let (mut m, ws, tab, a) = model_with_one_tab();
        let (t2, p2, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.attach_client(client("c_0001", &ws, &tab, &a));
        m.attach_client(ClientView {
            focus: Focus::Region(RegionKind::Overlay),
            ..client("c_0002", &ws, &tab, &a)
        });
        m.attach_client(client("c_0003", &ws, &t2, &p2));

        let (new, _) = m
            .split_pane(&a, Direction::Right, PathBuf::from("/x"))
            .unwrap();
        assert_eq!(
            m.client(&ClientId("c_0001".into())).unwrap().focus,
            Focus::Pane(new)
        );
        assert_eq!(
            m.client(&ClientId("c_0002".into())).unwrap().focus,
            Focus::Region(RegionKind::Overlay),
            "a client focused on a region keeps its focus"
        );
        assert_eq!(
            m.client(&ClientId("c_0003".into())).unwrap().focus,
            Focus::Pane(p2),
            "a client on another tab does not follow"
        );
    }

    #[test]
    fn close_pane_moves_only_the_clients_that_were_on_the_closed_pane() {
        let (mut m, ws, tab, a, b, c) = three_panes();
        let (t2, p2, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.attach_client(client("c_0001", &ws, &tab, &c));
        m.attach_client(client("c_0002", &ws, &tab, &a));
        m.attach_client(client("c_0003", &ws, &t2, &p2));

        m.close_pane(&c).unwrap();
        assert_eq!(m.tab(&tab).unwrap().focused, b);
        assert_eq!(
            m.client(&ClientId("c_0001".into())).unwrap().focus,
            Focus::Pane(b),
            "the client on the closed pane has to be moved off it"
        );
        assert_eq!(
            m.client(&ClientId("c_0002".into())).unwrap().focus,
            Focus::Pane(a),
            "a client on another pane of the same tab stays put"
        );
        assert_eq!(
            m.client(&ClientId("c_0003".into())).unwrap().focus,
            Focus::Pane(p2),
            "a client on another tab stays put"
        );
    }

    #[test]
    fn focus_pane_moves_the_pane_focused_clients_on_that_tab() {
        let (mut m, ws, tab, a, b, _c) = three_panes();
        let (t2, p2, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.attach_client(client("c_0001", &ws, &tab, &a));
        m.attach_client(ClientView {
            focus: Focus::Region(RegionKind::Overlay),
            ..client("c_0002", &ws, &tab, &a)
        });
        m.attach_client(client("c_0003", &ws, &t2, &p2));

        m.focus_pane(&b).unwrap();
        assert_eq!(
            m.client(&ClientId("c_0001".into())).unwrap().focus,
            Focus::Pane(b)
        );
        assert_eq!(
            m.client(&ClientId("c_0002".into())).unwrap().focus,
            Focus::Region(RegionKind::Overlay),
            "a client focused on a region keeps its focus"
        );
        assert_eq!(
            m.client(&ClientId("c_0003".into())).unwrap().focus,
            Focus::Pane(p2),
            "a client on another tab does not follow"
        );
    }

    #[test]
    fn select_tab_dismisses_the_clients_overlay() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        let (t2, _, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.attach_client(ClientView {
            overlay: Some(Overlay::Help),
            ..client("c_0001", &ws, &tab, &pane)
        });

        m.select_tab(&ClientId("c_0001".into()), &t2).unwrap();
        assert_eq!(
            m.client(&ClientId("c_0001".into())).unwrap().overlay,
            None,
            "an overlay drawn over the old tab must not survive the move"
        );
    }

    #[test]
    fn add_folder_project_seeds_last_workspace_once() {
        let mut m = Model::new(7);
        assert_eq!(m.last_workspace, None);
        let (_, ws1, _) = m.add_folder_project(PathBuf::from("/a")).unwrap();
        assert_eq!(m.last_workspace, Some(ws1.clone()));
        m.add_folder_project(PathBuf::from("/b")).unwrap();
        assert_eq!(
            m.last_workspace,
            Some(ws1),
            "a later project does not steal it"
        );
    }

    #[test]
    fn detach_client_reports_nothing_when_that_client_was_not_attached() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        m.attach_client(client("c_0001", &ws, &tab, &pane));
        assert_eq!(
            m.detach_client(&ClientId("c_0009".into())),
            Vec::<Event>::new(),
            "no client of that id was ever attached"
        );
        assert_eq!(
            m.detach_client(&ClientId("c_0001".into())),
            vec![Event::ClientDetached {
                client: ClientId("c_0001".into())
            }]
        );
        assert_eq!(
            m.detach_client(&ClientId("c_0001".into())),
            Vec::<Event>::new(),
            "and a second detach reports nothing"
        );
    }

    #[test]
    fn retired_ids_stop_at_the_capacity_bound() {
        // The bound is the whole reason the retired set is safe: an unbounded one would in
        // the end occupy every value `hex4` can draw and turn id reuse into a hang.
        let mut m = Model::new(1);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        for _ in 0..RETIRED_CAPACITY {
            let (t, _, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
            m.close_tab(&t).unwrap();
        }
        assert_eq!(
            m.retired.len(),
            RETIRED_CAPACITY,
            "{} closes retire two ids each, so the queue must have evicted",
            RETIRED_CAPACITY
        );

        // And from a state above the bound, which the full-id-space test constructs in this
        // same module, one retire still brings the queue back under it.
        m.retired = (0..=u16::MAX).map(|v| format!("t_{v:04x}")).collect();
        m.retire("t_0000_over".into());
        assert_eq!(
            m.retired.len(),
            RETIRED_CAPACITY,
            "the bound holds from any starting state, not only from below it"
        );
    }

    #[test]
    fn models_are_equal_when_their_public_content_matches() {
        let (mut a, ws, tab, pane) = model_with_one_tab();
        let mut b = a.clone();
        assert_eq!(a, b);

        // `clients` is public, so a difference there is one a reader outside the module can
        // see, and equality reports it even though the field is never persisted.
        a.attach_client(client("c_0001", &ws, &tab, &pane));
        assert_ne!(a, b, "an attached client is public content");
        b.attach_client(client("c_0001", &ws, &tab, &pane));
        assert_eq!(a, b);

        // The private machinery is excluded: a different generator, a different activity
        // counter and a different retired set leave the two equal.
        b.reseed(99);
        b.activity_seq += 5;
        b.retire("p_dead".into());
        assert_eq!(a, b, "private machinery is not part of equality");

        b.last_workspace = None;
        assert_ne!(a, b, "last_workspace is public content");
    }

    #[test]
    fn next_id_reports_a_full_id_space_instead_of_spinning() {
        // Reaches into `retired` because filling a prefix's 65536 values through the public
        // API would mean holding 65536 live objects. What is pinned is the bound itself: an
        // unbounded retry loop hangs here rather than returning.
        let mut m = Model::new(7);
        m.retired = (0..=u16::MAX).map(|v| format!("c_{v:04x}")).collect();
        let err = m.next_id("c").unwrap_err();
        assert_eq!(err.code, crate::api::ErrorCode::Internal);
        assert_eq!(
            err.message,
            "no free client id: every draw hit an id already in use or recently closed, so detach a client",
            "the message names the object, states only that the draws collided, and gives an action that frees one"
        );
        // Another prefix is a separate space and is unaffected.
        assert!(m.next_id("p").unwrap().starts_with("p_"));
    }

    #[test]
    fn ids_are_unique_within_the_model() {
        let mut m = Model::new(1);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let (t, p, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
            assert!(seen.insert(t.0));
            assert!(seen.insert(p.0));
        }
    }
}
