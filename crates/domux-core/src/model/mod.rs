//! The Model: the root of all shared state, owned by the core task. Every mutation is a
//! method here that returns the events it produced. Nothing outside this crate writes fields.

pub mod focus;
pub mod layout;

pub use focus::{ConfirmKind, Focus, Overlay, PromptKind, RegionKind, TextInput};
pub use layout::{Direction, LayoutNode, Pane, PaneContent, Rect, SplitDir};

use crate::api::{ApiError, Event};
use crate::ids::{ClientId, IdGen, PaneId, ProjectId, TabId, WorkspaceId};
use crate::names::BIN_NAME;
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
    /// The state a new client's sidebar starts in. Every `sidebar.toggle` updates it
    /// (roadmap decision 4). This is a deliberate exception to "per-client state is not
    /// persisted": a layout toggle that forgets itself on every attach is a daily
    /// irritation.
    #[serde(default)]
    pub sidebar_open: bool,
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
            sidebar_open,
            idgen: _,
            activity_seq: _,
            retired: _,
        } = self;
        *projects == other.projects
            && *clients == other.clients
            && *last_workspace == other.last_workspace
            && *sidebar_open == other.sidebar_open
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

    /// V1's `isEmptySlot` rule (`picker.go`), adapted: a slot with no name, no pull
    /// request, no agent and a branch equal to its handle draws as one line in the Projects
    /// box (interface spec 5.2 and 12.23). `main` is never one. An unknown branch is not one
    /// either: absent is not "equal to the handle" (principle 4).
    ///
    /// M3 adds "no agent" with a third argument; two are what M2 can know.
    pub fn is_untouched(&self, branch: Option<&str>, has_pr: bool) -> bool {
        self.handle != WorkspaceHandle::Main
            && self.name.is_none()
            && !has_pr
            && branch == Some(self.handle.to_string().as_str())
    }

    /// True when the branch line would only repeat what line 1 already says.
    pub fn branch_is_handle(&self, branch: &str) -> bool {
        branch == self.handle.to_string()
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

/// The narrowest screen that still gets the sidebar: 38 for the sidebar, 1 for the gap,
/// 80 for one useful pane, 1 spare (interface spec 12.1). Below this the sidebar hides
/// itself for that client and the top bar returns; the remembered state does not change.
pub const SIDEBAR_MIN_COLS: u16 = 120;

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
    /// This client asked for the sidebar on a screen too narrow to show it on its own, so the
    /// width rule does not apply here: `leader b` shows it at any width (interface spec 12.1).
    /// Cleared when the sidebar is hidden, because there is then nothing left to override.
    #[serde(default)]
    pub sidebar_forced: bool,
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
    /// The row the keys act on while focus is in the Projects box. `None` means the fill is
    /// the current row: the workspace this client is in (domain model, section 3.3).
    #[serde(default)]
    pub projects_cursor: Option<WorkspaceId>,
    /// The first visible line inside the Projects box, so scrolling moves as little as it
    /// can when the cursor leaves the view.
    #[serde(default)]
    pub projects_scroll: u16,
    /// True while `/` is being typed into. `filter` holds the text either way.
    #[serde(default)]
    pub filtering: bool,
    /// The text an open overlay is editing: the name box in M2.
    #[serde(default)]
    pub input: TextInput,
    /// The overlay this one was opened over, so Esc returns to it (interface spec 12.7).
    #[serde(default)]
    pub overlay_under: Option<Overlay>,
    /// The last result of an action, shown in the hint row or the footer.
    #[serde(default)]
    pub pill: Option<Pill>,
}

impl ClientView {
    /// Whether this client draws the sidebar now.
    ///
    /// Two bits, not one (interface spec 12.1). `sidebar_open` is the remembered intent, and
    /// it is the server's: `leader b` flips it and every client follows. The auto-hide is
    /// this client's own, and it is an override rather than part of the intent, so a screen
    /// that grew wide again shows the sidebar the reader never closed. `sidebar_forced` is
    /// the reader overriding the override: they asked for it on a narrow screen and got it.
    ///
    /// One bit cannot carry this, because it cannot tell "hidden because the screen is
    /// narrow" from "hidden because you said so", and those come back differently.
    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_open && (self.size.cols >= SIDEBAR_MIN_COLS || self.sidebar_forced)
    }

    /// Opens `overlay` over whatever is open, keeping one level underneath.
    pub fn push_overlay(&mut self, overlay: Overlay) {
        self.overlay_under = self.overlay.take();
        self.overlay = Some(overlay);
    }

    /// Closes the top overlay and returns the one that is open now.
    pub fn pop_overlay(&mut self) -> Option<Overlay> {
        self.overlay = self.overlay_under.take();
        self.input = TextInput::new("");
        self.filtering = false;
        self.overlay.clone()
    }
}

/// A one-line result in the hint row or the footer: green when it worked, red when it was
/// refused (interface spec 7.3). It clears on the next key in a box or after
/// `PILL_SECONDS` (interface spec 12.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Pill {
    pub text: String,
    pub ok: bool,
    /// RFC 3339, from the server's clock.
    pub at: String,
}

pub const PILL_SECONDS: u64 = 6;

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
            sidebar_open: false,
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

    /// Registers a project at `root` with its `main` workspace and no tabs. The project's
    /// name is the folder's last path component, and its `main` workspace is the checkout at
    /// `root` itself.
    ///
    /// Fallible for one reason: `next_id` is. It reports a full id space rather than
    /// reissuing a live id, so every caller of it propagates.
    ///
    /// Private, and it reports no events: the two public constructors below differ only in
    /// the kind they pass and the events they report, and this is everything they share.
    fn add_project(
        &mut self,
        root: &Path,
        kind: ProjectKind,
    ) -> Result<(ProjectId, WorkspaceId, String), ApiError> {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let pid = ProjectId(self.next_id("pr")?);
        let wid = WorkspaceId(self.next_id("w")?);
        self.projects.push(Project {
            id: pid.clone(),
            name: name.clone(),
            root: root.to_path_buf(),
            kind,
            workspaces: vec![Workspace {
                id: wid.clone(),
                handle: WorkspaceHandle::Main,
                name: None,
                path: root.to_path_buf(),
                tabs: Vec::new(),
                last_tab: None,
            }],
        });
        if self.last_workspace.is_none() {
            self.last_workspace = Some(wid.clone());
        }
        Ok((pid, wid, name))
    }

    /// Registers a plain folder as a project. Returns no events in M1 (`project.added` is an
    /// M2 event), and fails only when the id space is full.
    pub fn add_folder_project(
        &mut self,
        root: PathBuf,
    ) -> Result<(ProjectId, WorkspaceId, Vec<Event>), ApiError> {
        let (pid, wid, _) = self.add_project(&root, ProjectKind::Folder)?;
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
            ApiError::not_found(format!(
                "tab {tab} does not exist; run {BIN_NAME} api tab.list"
            ))
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
                ApiError::not_found(format!(
                    "tab {tab} does not exist; run {BIN_NAME} api tab.list"
                ))
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
            ApiError::not_found(format!(
                "tab {tab} does not exist; run {BIN_NAME} api tab.list"
            ))
        })?;
        let ws_id = self
            .workspace_of_tab(tab)
            .map(|w| w.id.clone())
            .expect("tab has a workspace");
        let c = self.client_mut(client).ok_or_else(|| {
            ApiError::not_found(format!(
                "client {client} is not attached; run {BIN_NAME} api server.info"
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
                "pane {pane} does not exist; run {BIN_NAME} api pane.list"
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
                "pane {pane} does not exist; run {BIN_NAME} api pane.list"
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
                "pane {pane} does not exist; run {BIN_NAME} api pane.list"
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
            ApiError::not_found(format!(
                "tab {tab} does not exist; run {BIN_NAME} api tab.list"
            ))
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
                "pane {pane} does not exist; run {BIN_NAME} api pane.list"
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
                    "tab {target:?} does not exist; run {BIN_NAME} api tab.list"
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
                "pane {target} does not exist; run {BIN_NAME} api pane.list"
            )))
        }
    }

    /// Registers a git repository as a project with its `main` workspace. `default_branch`
    /// is the short name `origin/HEAD` points at, falling back to `main`; the server reads
    /// it with `git::default_branch` before calling this.
    ///
    /// It does not check whether a project is registered at `root` already, and registering
    /// one twice gives two projects. `project.add` is what makes that idempotent, by asking
    /// `project_at` first: it is the caller that has resolved the canonical path, and one
    /// rule belongs in one place.
    ///
    /// Fallible for the reason `add_folder_project` is: `next_id` is.
    pub fn add_git_project(
        &mut self,
        root: PathBuf,
        default_branch: String,
    ) -> Result<(ProjectId, WorkspaceId, Vec<Event>), ApiError> {
        let (pid, wid, name) = self.add_project(&root, ProjectKind::Git { default_branch })?;
        Ok((
            pid.clone(),
            wid,
            vec![Event::ProjectAdded {
                project: pid,
                name,
                root,
            }],
        ))
    }

    /// A project already registered at `root`, so `project.add` is idempotent.
    pub fn project_at(&self, root: &Path) -> Option<&Project> {
        self.projects.iter().find(|p| p.root == root)
    }

    pub fn project(&self, id: &ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| &p.id == id)
    }

    pub fn project_mut(&mut self, id: &ProjectId) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| &p.id == id)
    }

    pub fn project_of_workspace_mut(&mut self, id: &WorkspaceId) -> Option<&mut Project> {
        self.projects
            .iter_mut()
            .find(|p| p.workspaces.iter().any(|w| &w.id == id))
    }

    /// The lowest N at or above 1 that no slot in this project holds and that `spoken_for`
    /// does not name. Numbers a delete freed come back; a number a live slot holds never
    /// moves (architecture spec 2).
    ///
    /// `spoken_for` is how the server adds the numbers a create has already chosen but not
    /// yet recorded. The model learns a slot when that create's job finishes, and the choice
    /// is made when the call arrives, so without it two creates in flight both pick this
    /// same number and both try to build it (decision record 0006). A caller with nothing in
    /// flight passes `|_| false`.
    pub fn lowest_free_slot(
        &self,
        project: &ProjectId,
        spoken_for: impl Fn(u32) -> bool,
    ) -> Result<u32, ApiError> {
        let p = self
            .project(project)
            .ok_or_else(|| ApiError::not_found(format!("no project with id {project}")))?;
        let taken: Vec<u32> = p
            .workspaces
            .iter()
            .filter_map(|w| match w.handle {
                WorkspaceHandle::Slot(n) => Some(n),
                WorkspaceHandle::Main => None,
            })
            .collect();
        Ok((1u32..)
            .find(|n| !taken.contains(n) && !spoken_for(*n))
            .expect("u32 is not exhausted"))
    }

    /// Adds the record for a slot whose worktree already exists on disk. `project.add` calls
    /// it for the worktrees it discovers and `workspace.create` for the one it just made.
    pub fn add_slot(
        &mut self,
        project: &ProjectId,
        slot: u32,
        path: PathBuf,
    ) -> Result<(WorkspaceId, Vec<Event>), ApiError> {
        // `next_id` is fallible, so this needs `?`. It also has to run before `project_mut`
        // borrows the model mutably.
        let wid = WorkspaceId(self.next_id("w")?);
        let p = self
            .project_mut(project)
            .ok_or_else(|| ApiError::not_found(format!("no project with id {project}")))?;
        if p.workspaces
            .iter()
            .any(|w| w.handle == WorkspaceHandle::Slot(slot))
        {
            return Err(ApiError::conflict(format!(
                "workspace-{slot} already exists in {}",
                p.name
            )));
        }
        p.workspaces.push(Workspace {
            id: wid.clone(),
            handle: WorkspaceHandle::Slot(slot),
            name: None,
            path: path.clone(),
            tabs: Vec::new(),
            last_tab: None,
        });
        p.workspaces.sort_by_key(|w| match w.handle {
            WorkspaceHandle::Main => 0,
            WorkspaceHandle::Slot(n) => n,
        });
        let project = project.clone();
        Ok((
            wid.clone(),
            vec![Event::WorkspaceCreated {
                project,
                workspace: wid,
                handle: format!("workspace-{slot}"),
                path,
            }],
        ))
    }

    /// Sets or clears a workspace's name. An empty or blank name clears it and the handle
    /// comes back (architecture spec 2).
    pub fn rename_workspace(
        &mut self,
        id: &WorkspaceId,
        name: Option<String>,
    ) -> Result<Vec<Event>, ApiError> {
        let w = self
            .workspace_mut(id)
            .ok_or_else(|| ApiError::not_found(format!("no workspace with id {id}")))?;
        let trimmed = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        if w.name == trimmed {
            return Ok(Vec::new());
        }
        w.name = trimmed.clone();
        Ok(vec![Event::WorkspaceRenamed {
            workspace: id.clone(),
            name: trimmed,
        }])
    }

    /// Removes a slot's record. Refuses `main`. The caller has already removed the worktree,
    /// or found the path gone (`prune_workspace`).
    pub fn remove_workspace(
        &mut self,
        id: &WorkspaceId,
    ) -> Result<(WorkspaceHandle, Vec<Event>), ApiError> {
        self.remove_workspace_inner(id, false)
    }

    /// The same, marked as a prune so subscribers can tell a deletion from a workspace whose
    /// path went away underneath (architecture spec 5).
    pub fn prune_workspace(
        &mut self,
        id: &WorkspaceId,
    ) -> Result<(WorkspaceHandle, Vec<Event>), ApiError> {
        self.remove_workspace_inner(id, true)
    }

    fn remove_workspace_inner(
        &mut self,
        id: &WorkspaceId,
        pruned: bool,
    ) -> Result<(WorkspaceHandle, Vec<Event>), ApiError> {
        let w = self
            .workspace(id)
            .ok_or_else(|| ApiError::not_found(format!("no workspace with id {id}")))?;
        if w.handle == WorkspaceHandle::Main {
            return Err(ApiError::refused(
                "main is the project's checkout and cannot be deleted; delete a workspace-N slot instead",
            ));
        }
        let handle = w.handle;
        // Every id that is about to stop existing, collected before the removal. Removing a
        // workspace takes its tabs and panes with it without going through `close_tab` and
        // `close_pane`, which is where those ids would normally be retired, so this path
        // retires them itself. Skipping that reopens exactly the stale-id aliasing
        // `retired` exists to prevent: a client still holding a closed pane's id would find
        // it pointing at a different pane after the id came back around.
        let doomed = Self::ids_under_workspace(w);
        let project = self
            .project_of_workspace(id)
            .expect("a workspace has a project")
            .id
            .clone();
        let p = self
            .project_of_workspace_mut(id)
            .expect("a workspace has a project");
        p.workspaces.retain(|w| &w.id != id);
        if self.last_workspace.as_ref() == Some(id) {
            self.last_workspace = self.first_workspace();
        }
        for gone in doomed {
            self.retire(gone);
        }
        Ok((
            handle,
            vec![Event::WorkspaceDeleted {
                project,
                workspace: id.clone(),
                handle: handle.to_string(),
                pruned,
            }],
        ))
    }

    /// The workspace's own id and every tab and pane id under it, as strings. Used by the
    /// removal paths to retire what they delete.
    fn ids_under_workspace(w: &Workspace) -> Vec<String> {
        let mut ids = vec![w.id.to_string()];
        for t in &w.tabs {
            ids.push(t.id.to_string());
            ids.extend(t.layout.panes().iter().map(|p| p.id.to_string()));
        }
        ids
    }

    /// Removes a project and every workspace record under it. The folder and its worktrees
    /// stay on disk (interface spec 12.8).
    pub fn remove_project(&mut self, id: &ProjectId) -> Result<Vec<Event>, ApiError> {
        let p = self
            .project(id)
            .ok_or_else(|| ApiError::not_found(format!("no project with id {id}")))?;
        let name = p.name.clone();
        let gone: Vec<WorkspaceId> = p.workspaces.iter().map(|w| w.id.clone()).collect();
        // The project id, and every workspace, tab and pane id under it. Same reason as
        // `remove_workspace_inner`: this path bypasses the close paths that retire.
        let mut doomed = vec![id.to_string()];
        for w in &p.workspaces {
            doomed.extend(Self::ids_under_workspace(w));
        }
        self.projects.retain(|p| &p.id != id);
        if self
            .last_workspace
            .as_ref()
            .is_some_and(|w| gone.contains(w))
        {
            self.last_workspace = self.first_workspace();
        }
        for doomed_id in doomed {
            self.retire(doomed_id);
        }
        Ok(vec![Event::ProjectRemoved {
            project: id.clone(),
            name,
        }])
    }

    /// Every workspace of one project, in handle order. The Projects box and
    /// `workspace.list` walk this.
    ///
    /// Written through `project` so the returned iterator borrows only the model: the
    /// project is looked up before the iterator is built, and `project`'s own lifetime does
    /// not have to outlive the walk.
    pub fn workspaces_of<'a>(&'a self, project: &ProjectId) -> impl Iterator<Item = &'a Workspace> {
        self.project(project)
            .into_iter()
            .flat_map(|p| p.workspaces.iter())
    }

    /// Id, handle, name, then branch, in that order (architecture spec section 2). The
    /// branch pass needs the facts, so the server passes them in; this signature takes the
    /// branch of each workspace as a lookup so `domux-core` stays free of the registry.
    pub fn resolve_workspace_with(
        &self,
        target: &str,
        branch_of: &dyn Fn(&WorkspaceId) -> Option<String>,
    ) -> Result<WorkspaceId, ApiError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(ApiError::invalid_params(
                "name a workspace: an id, a handle such as workspace-1, a name, or a branch",
            ));
        }
        let all: Vec<&Workspace> = self
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .collect();
        if let Some(w) = all.iter().find(|w| w.id.as_str() == target) {
            return Ok(w.id.clone());
        }
        for pass in 0..3 {
            let hits: Vec<&&Workspace> = all
                .iter()
                .filter(|w| match pass {
                    0 => w.handle.to_string().eq_ignore_ascii_case(target),
                    1 => w
                        .name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(target)),
                    _ => branch_of(&w.id).is_some_and(|b| b == target),
                })
                .collect();
            match hits.len() {
                0 => continue,
                1 => return Ok(hits[0].id.clone()),
                _ => {
                    let names: Vec<String> = hits
                        .iter()
                        .filter_map(|w| self.project_of_workspace(&w.id))
                        .map(|p| p.name.clone())
                        .collect();
                    let ids: Vec<String> = hits.iter().map(|w| w.id.to_string()).collect();
                    let count = if names.len() == 2 {
                        "two".to_string()
                    } else {
                        names.len().to_string()
                    };
                    // `ambiguous(message, candidates)` puts the ids in `data` itself, as a
                    // bare array. There is no `with_data`.
                    return Err(ApiError::ambiguous(
                        format!(
                            "{target} is in {count} projects: {}; name the project or use an id",
                            names.join(", ")
                        ),
                        ids,
                    ));
                }
            }
        }
        Err(ApiError::not_found(format!(
            "no workspace called {target}; run {BIN_NAME} workspace list to see them"
        )))
    }

    /// The common case: no branch facts, so id, handle and name only.
    pub fn resolve_workspace(&self, target: &str) -> Result<WorkspaceId, ApiError> {
        self.resolve_workspace_with(target, &|_| None)
    }

    /// The project an id or a name names. Ids first, then names without case, which is the
    /// order `resolve_workspace_with` uses and for the same reason: an id is exact, so a
    /// project someone named `pr_8f2a` cannot shadow the project with that id.
    ///
    /// Two projects can share a name - the name is the folder's last component, and two
    /// checkouts of the same repository in different parents have the same one - so a name
    /// that matches twice answers `ambiguous` with the ids rather than picking the first.
    pub fn resolve_project(&self, target: &str) -> Result<ProjectId, ApiError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(ApiError::invalid_params("name a project: an id or a name"));
        }
        if let Some(p) = self.projects.iter().find(|p| p.id.as_str() == target) {
            return Ok(p.id.clone());
        }
        let hits: Vec<&Project> = self
            .projects
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(target))
            .collect();
        match hits.len() {
            1 => Ok(hits[0].id.clone()),
            0 => Err(ApiError::not_found(format!(
                "no project called {target}; run {BIN_NAME} project list to see them"
            ))),
            _ => Err(ApiError::ambiguous(
                format!(
                    "{} projects are called {target}: {}; use an id",
                    hits.len(),
                    hits.iter()
                        .map(|p| p.root.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                hits.iter().map(|p| p.id.to_string()).collect(),
            )),
        }
    }

    /// Sets the remembered sidebar state and answers with it.
    pub fn set_sidebar_open(&mut self, open: bool) -> bool {
        self.sidebar_open = open;
        self.sidebar_open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ErrorCode, Event};
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
            sidebar_forced: false,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            filtering: false,
            input: TextInput::new(""),
            overlay_under: None,
            pill: None,
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

    fn git_model() -> (Model, ProjectId, WorkspaceId) {
        let mut m = Model::new(7);
        let (pid, main, _) = m
            .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
            .unwrap();
        (m, pid, main)
    }

    #[test]
    fn a_git_project_takes_its_name_from_the_folder_and_starts_with_main_only() {
        let (m, pid, main) = git_model();
        let p = m.project(&pid).unwrap();
        assert_eq!(p.name, "audrey-app");
        assert_eq!(
            p.kind,
            ProjectKind::Git {
                default_branch: "main".into()
            }
        );
        assert_eq!(
            p.workspaces.len(),
            1,
            "a slot is created only on workspace.create"
        );
        assert_eq!(p.workspaces[0].id, main);
        assert_eq!(p.workspaces[0].handle, WorkspaceHandle::Main);
        assert_eq!(p.workspaces[0].path, PathBuf::from("/repo/audrey-app"));
    }

    /// Registering the same root twice is not refused here. `project.add` answers that, by
    /// asking `project_at` before it calls this; the model would otherwise hold the same
    /// rule twice, and the canonical path a root resolves to is only known once the git job
    /// has run. So the absence of a refusal below is the design, not an oversight.
    #[test]
    fn add_git_project_reports_project_added_and_registers_the_root() {
        let mut m = Model::new(7);
        let (pid, _, events) = m
            .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
            .unwrap();
        assert_eq!(
            events,
            vec![Event::ProjectAdded {
                project: pid,
                name: "audrey-app".into(),
                root: PathBuf::from("/repo/audrey-app")
            }]
        );
        assert!(m.project_at(&PathBuf::from("/repo/audrey-app")).is_some());
    }

    #[test]
    fn slots_take_the_lowest_free_number_and_never_renumber() {
        let (mut m, pid, _) = git_model();
        assert_eq!(m.lowest_free_slot(&pid, |_| false).unwrap(), 1);
        let (w1, _) = m
            .add_slot(
                &pid,
                1,
                PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1"),
            )
            .unwrap();
        let (w2, _) = m
            .add_slot(
                &pid,
                2,
                PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-2"),
            )
            .unwrap();
        assert_eq!(m.lowest_free_slot(&pid, |_| false).unwrap(), 3);
        m.remove_workspace(&w1).unwrap();
        assert_eq!(
            m.lowest_free_slot(&pid, |_| false).unwrap(),
            1,
            "a freed number comes back"
        );
        assert_eq!(
            m.workspace(&w2).unwrap().handle,
            WorkspaceHandle::Slot(2),
            "the survivor keeps its number"
        );
        assert!(
            m.add_slot(&pid, 2, PathBuf::from("/x")).is_err(),
            "a taken number is refused"
        );
    }

    /// A number a create has chosen but not yet recorded is skipped, and the search carries
    /// on past it rather than stopping at the number after it.
    ///
    /// The model holds slot 2 and `spoken_for` names 1, so the three implementations that
    /// could be here give three different answers: ignoring `spoken_for` says 1, adding one
    /// to the lowest free number says 2, and looking at both says 3. A fixture where the
    /// model held nothing could not tell the first two apart from the third.
    #[test]
    fn a_slot_number_another_call_has_spoken_for_is_skipped() {
        let (mut m, pid, _) = git_model();
        m.add_slot(
            &pid,
            2,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-2"),
        )
        .unwrap();
        assert_eq!(m.lowest_free_slot(&pid, |n| n == 1).unwrap(), 3);
        assert_eq!(
            m.lowest_free_slot(&pid, |_| false).unwrap(),
            1,
            "and nothing spoken for leaves the number free"
        );
    }

    #[test]
    fn removing_main_is_refused_and_removing_a_slot_reports_the_event() {
        let (mut m, pid, main) = git_model();
        let (w1, _) = m
            .add_slot(
                &pid,
                1,
                PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1"),
            )
            .unwrap();
        let err = m.remove_workspace(&main).unwrap_err();
        assert_eq!(err.code, ErrorCode::Refused);
        assert_eq!(
            err.message,
            "main is the project's checkout and cannot be deleted; delete a workspace-N slot instead"
        );
        let (handle, events) = m.remove_workspace(&w1).unwrap();
        assert_eq!(handle, WorkspaceHandle::Slot(1));
        assert_eq!(
            events,
            vec![Event::WorkspaceDeleted {
                project: pid,
                workspace: w1,
                handle: "workspace-1".into(),
                pruned: false
            }]
        );
    }

    #[test]
    fn removing_a_workspace_retires_its_id_and_every_id_under_it() {
        // Stale-id aliasing is what `retire` exists to prevent, and a workspace removal takes
        // its tabs and panes with it without going through the tab and pane close paths that
        // normally do the retiring. So this path retires them itself.
        let (mut m, pid, _) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let (tab, pane, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
        m.remove_workspace(&w1).unwrap();
        for id in [w1.as_str(), tab.as_str(), pane.as_str()] {
            assert!(
                m.retired.iter().any(|r| r == id),
                "{id} must not be reissued"
            );
        }
    }

    #[test]
    fn removing_a_project_retires_its_id_and_every_id_under_it() {
        let (mut m, pid, main) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let (tab, pane, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
        m.remove_project(&pid).unwrap();
        for id in [
            pid.as_str(),
            main.as_str(),
            w1.as_str(),
            tab.as_str(),
            pane.as_str(),
        ] {
            assert!(
                m.retired.iter().any(|r| r == id),
                "{id} must not be reissued"
            );
        }
    }

    #[test]
    fn resolve_workspace_takes_an_id_a_handle_a_name_or_a_branch_in_that_order() {
        let (mut m, pid, main) = git_model();
        let (w1, _) = m
            .add_slot(
                &pid,
                1,
                PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1"),
            )
            .unwrap();
        m.rename_workspace(&w1, Some("auth cleanup".into()))
            .unwrap();
        assert_eq!(m.resolve_workspace(w1.as_str()).unwrap(), w1);
        assert_eq!(m.resolve_workspace("workspace-1").unwrap(), w1);
        assert_eq!(m.resolve_workspace("auth cleanup").unwrap(), w1);
        assert_eq!(m.resolve_workspace("main").unwrap(), main);
        assert_eq!(
            m.resolve_workspace("AUTH CLEANUP").unwrap(),
            w1,
            "names match without case"
        );
        let err = m.resolve_workspace("nope").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(
            err.message,
            "no workspace called nope; run domux2 workspace list to see them"
        );
    }

    #[test]
    fn an_ambiguous_target_lists_its_candidates() {
        let mut m = Model::new(7);
        let (a, _, _) = m
            .add_git_project(PathBuf::from("/repo/one"), "main".into())
            .unwrap();
        let (b, _, _) = m
            .add_git_project(PathBuf::from("/repo/two"), "main".into())
            .unwrap();
        let (wa, _) = m
            .add_slot(
                &a,
                1,
                PathBuf::from("/repo/one/.domux/worktrees/workspace-1"),
            )
            .unwrap();
        let (wb, _) = m
            .add_slot(
                &b,
                1,
                PathBuf::from("/repo/two/.domux/worktrees/workspace-1"),
            )
            .unwrap();
        let err = m.resolve_workspace("workspace-1").unwrap_err();
        assert_eq!(err.code, ErrorCode::Ambiguous);
        assert_eq!(
            err.message,
            "workspace-1 is in two projects: one, two; name the project or use an id"
        );
        // M1's `ApiError::ambiguous(message, candidates)` puts the candidate ids in `data` as a
        // bare array. That is the wire shape every ambiguous error already has, so M2 matches it
        // rather than wrapping the array in an object for this one call site.
        assert_eq!(
            err.data.unwrap(),
            serde_json::json!([wa.as_str(), wb.as_str()])
        );
        assert_eq!(
            m.resolve_workspace("main").unwrap_err().code,
            ErrorCode::Ambiguous
        );
    }

    #[test]
    fn naming_a_workspace_reports_it_and_an_empty_name_clears_it() {
        let (mut m, pid, _) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let events = m
            .rename_workspace(&w1, Some("  auth cleanup  ".into()))
            .unwrap();
        assert_eq!(
            m.workspace(&w1).unwrap().name.as_deref(),
            Some("auth cleanup"),
            "names are trimmed"
        );
        assert_eq!(
            events,
            vec![Event::WorkspaceRenamed {
                workspace: w1.clone(),
                name: Some("auth cleanup".into())
            }]
        );
        m.rename_workspace(&w1, Some("   ".into())).unwrap();
        assert_eq!(m.workspace(&w1).unwrap().name, None, "whitespace is empty");
        assert_eq!(
            m.workspace(&w1).unwrap().display_name(),
            "workspace-1",
            "the handle comes back"
        );
        m.rename_workspace(&w1, None).unwrap();
        assert_eq!(m.workspace(&w1).unwrap().name, None);
    }

    #[test]
    fn an_untouched_slot_has_no_name_no_pull_request_and_a_branch_equal_to_its_handle() {
        let (mut m, pid, main) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let w = m.workspace(&w1).unwrap();
        assert!(w.is_untouched(Some("workspace-1"), false));
        assert!(
            !w.is_untouched(Some("feat/x"), false),
            "a renamed branch touches it"
        );
        assert!(
            !w.is_untouched(Some("workspace-1"), true),
            "a pull request touches it"
        );
        assert!(
            !w.is_untouched(None, false),
            "an unknown branch is not an untouched slot"
        );
        m.rename_workspace(&w1, Some("auth cleanup".into()))
            .unwrap();
        assert!(
            !m.workspace(&w1)
                .unwrap()
                .is_untouched(Some("workspace-1"), false),
            "a name touches it"
        );
        assert!(
            !m.workspace(&main)
                .unwrap()
                .is_untouched(Some("main"), false),
            "main is never an untouched slot"
        );
    }

    #[test]
    fn removing_a_project_takes_its_workspaces_and_reports_one_event() {
        let (mut m, pid, _) = git_model();
        m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let events = m.remove_project(&pid).unwrap();
        assert_eq!(
            events,
            vec![Event::ProjectRemoved {
                project: pid,
                name: "audrey-app".into()
            }]
        );
        assert!(m.projects.is_empty());
        assert_eq!(
            m.last_workspace, None,
            "the pointer into a removed project is cleared"
        );
    }

    #[test]
    fn removing_a_project_leaves_a_last_workspace_in_another_project_alone() {
        // `remove_project` moves `last_workspace` only when it pointed into the project
        // being removed. An unconditional reset passes every other test in this module and
        // still moves the user: working in another project's `workspace-1`, they remove a
        // project they are not in, and their next attach lands in that other project's
        // `main` instead of the slot they left, with nothing said about it.
        let (mut m, a, _) = git_model();
        let (b, b_main, _) = m
            .add_git_project(PathBuf::from("/repo/other"), "main".into())
            .unwrap();
        let (b_w1, _) = m.add_slot(&b, 1, PathBuf::from("/other/w1")).unwrap();

        m.last_workspace = Some(b_w1.clone());
        m.remove_project(&a).unwrap();
        assert_eq!(
            m.last_workspace,
            Some(b_w1),
            "a pointer into a project that survives is not moved"
        );

        let (c, c_main, _) = m
            .add_git_project(PathBuf::from("/repo/third"), "main".into())
            .unwrap();
        m.last_workspace = Some(c_main);
        m.remove_project(&c).unwrap();
        assert_eq!(
            m.last_workspace,
            Some(b_main),
            "and a pointer into the removed project moves to a workspace that is still there"
        );
    }

    #[test]
    fn the_sidebar_state_lives_on_the_model_and_hides_itself_on_a_narrow_screen() {
        let mut m = Model::new(7);
        assert!(!m.sidebar_open, "a fresh model starts with the top bar");
        // `assert_eq!(.., true)` is what the plan wrote; clippy's `bool_assert_comparison`
        // is denied in CI, so this says the same thing the way the lint asks for, and says
        // it in both directions so a setter that ignored its argument would be caught.
        assert!(
            m.set_sidebar_open(true),
            "it answers with the state it just set"
        );
        assert!(m.sidebar_open);
        assert!(!m.set_sidebar_open(false));
        assert!(!m.sidebar_open);
        m.set_sidebar_open(true);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let (tab, pane, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let mut view = client("c_0001", &ws, &tab, &pane);
        view.sidebar_open = true;
        view.size = Size {
            cols: 120,
            rows: 24,
        };
        assert!(view.sidebar_visible());
        view.size = Size {
            cols: 119,
            rows: 24,
        };
        assert!(
            !view.sidebar_visible(),
            "below SIDEBAR_MIN_COLS the sidebar hides itself"
        );
        assert!(
            view.sidebar_open,
            "auto-hide never changes the remembered state"
        );
        view.sidebar_open = false;
        view.size = Size {
            cols: 200,
            rows: 50,
        };
        assert!(
            !view.sidebar_visible(),
            "a sidebar you hid stays hidden at any width"
        );
    }

    #[test]
    fn an_overlay_pushed_over_another_one_comes_back_when_it_closes() {
        let mut m = Model::new(7);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let (tab, pane, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let mut view = client("c_0001", &ws, &tab, &pane);
        view.push_overlay(Overlay::Switcher);
        view.push_overlay(Overlay::NameWorkspace(ws.clone()));
        assert_eq!(view.overlay, Some(Overlay::NameWorkspace(ws.clone())));
        assert_eq!(view.overlay_under, Some(Overlay::Switcher));
        assert_eq!(
            view.pop_overlay(),
            Some(Overlay::Switcher),
            "the switcher is open again"
        );
        assert_eq!(view.overlay, Some(Overlay::Switcher));
        assert_eq!(view.overlay_under, None);
        assert_eq!(view.pop_overlay(), None);
        assert_eq!(view.overlay, None);
    }

    #[test]
    fn removing_a_workspace_moves_last_workspace_off_it() {
        let (mut m, pid, main) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let (w2, _) = m.add_slot(&pid, 2, PathBuf::from("/w2")).unwrap();
        m.last_workspace = Some(w1.clone());
        m.remove_workspace(&w1).unwrap();
        assert_eq!(
            m.last_workspace,
            Some(main.clone()),
            "a pointer into the removed workspace names a workspace that is gone"
        );
        m.remove_workspace(&w2).unwrap();
        assert_eq!(
            m.last_workspace,
            Some(main),
            "removing another workspace leaves it alone"
        );
    }

    #[test]
    fn slots_read_in_handle_order_whatever_order_they_were_added_in() {
        let (mut m, pid, _) = git_model();
        m.add_slot(&pid, 2, PathBuf::from("/w2")).unwrap();
        m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let (other, _, _) = m
            .add_git_project(PathBuf::from("/repo/other"), "main".into())
            .unwrap();
        m.add_slot(&other, 1, PathBuf::from("/other/w1")).unwrap();

        let handles: Vec<String> = m
            .workspaces_of(&pid)
            .map(|w| w.handle.to_string())
            .collect();
        assert_eq!(
            handles,
            vec!["main", "workspace-1", "workspace-2"],
            "the Projects box reads this order straight out of the model, so the model holds it"
        );
        assert_eq!(
            m.workspaces_of(&other).count(),
            2,
            "another project's workspaces are its own"
        );

        let missing = ProjectId("pr_0000".into());
        assert_eq!(m.workspaces_of(&missing).count(), 0);
        assert_eq!(
            m.lowest_free_slot(&missing, |_| false).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            m.add_slot(&missing, 1, PathBuf::from("/x"))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn resolve_workspace_answers_with_a_handle_before_a_name_and_a_name_before_a_branch() {
        // The three passes only differ where two of them match different workspaces, which
        // is the state this builds: the slot is named `main` while another workspace has
        // that handle, and then named after another workspace's branch.
        let (mut m, pid, main) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        m.rename_workspace(&w1, Some("main".into())).unwrap();
        assert_eq!(m.resolve_workspace("main").unwrap(), main);

        m.rename_workspace(&w1, Some("feature".into())).unwrap();
        let branch_of =
            |id: &WorkspaceId| Some(if id == &main { "feature" } else { "feat/auth" }.to_string());
        assert_eq!(
            m.resolve_workspace_with("feature", &branch_of).unwrap(),
            w1,
            "a name answers before a branch"
        );
        assert_eq!(
            m.resolve_workspace_with("feat/auth", &branch_of).unwrap(),
            w1,
            "and a branch answers when neither a handle nor a name matches"
        );
        assert_eq!(
            m.resolve_workspace("feat/auth").unwrap_err().code,
            ErrorCode::NotFound,
            "resolve_workspace knows no branches, so it cannot match one"
        );
    }

    #[test]
    fn resolve_workspace_trims_its_target_and_refuses_an_empty_one() {
        let (m, _, main) = git_model();
        assert_eq!(m.resolve_workspace("  main  ").unwrap(), main);
        for target in ["", "   "] {
            let err = m.resolve_workspace(target).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidParams);
            assert_eq!(
                err.message,
                "name a workspace: an id, a handle such as workspace-1, a name, or a branch"
            );
        }
    }

    #[test]
    fn resolve_project_takes_an_id_before_a_name_and_ignores_case() {
        let mut m = Model::new(3);
        let (one, _, _) = m
            .add_git_project(PathBuf::from("/code/audrey-app"), "main".into())
            .unwrap();
        let (two, _, _) = m.add_folder_project(PathBuf::from("/code/notes")).unwrap();
        assert_eq!(m.resolve_project(one.as_str()).unwrap(), one);
        assert_eq!(m.resolve_project("  AUDREY-app ").unwrap(), one);
        assert_eq!(m.resolve_project("notes").unwrap(), two);
        // An id is exact, so a project someone named after another project's id is not what
        // that id resolves to.
        m.project_mut(&two).unwrap().name = one.to_string();
        assert_eq!(
            m.resolve_project(one.as_str()).unwrap(),
            one,
            "the id wins over a name that spells it"
        );
    }

    #[test]
    fn resolve_project_refuses_an_empty_target_a_missing_one_and_a_shared_name() {
        let mut m = Model::new(3);
        let empty = m.resolve_project("  ").unwrap_err();
        assert_eq!(empty.code, ErrorCode::InvalidParams);
        assert_eq!(empty.message, "name a project: an id or a name");
        let missing = m.resolve_project("audrey-app").unwrap_err();
        assert_eq!(missing.code, ErrorCode::NotFound);
        assert_eq!(
            missing.message,
            "no project called audrey-app; run domux2 project list to see them"
        );
        // Two checkouts of one repository under different parents have the same name, which
        // is the folder's last component.
        let (one, _, _) = m
            .add_git_project(PathBuf::from("/work/audrey-app"), "main".into())
            .unwrap();
        let (two, _, _) = m
            .add_git_project(PathBuf::from("/spike/audrey-app"), "main".into())
            .unwrap();
        let shared = m.resolve_project("audrey-app").unwrap_err();
        assert_eq!(shared.code, ErrorCode::Ambiguous);
        assert_eq!(
            shared.message,
            "2 projects are called audrey-app: /work/audrey-app, /spike/audrey-app; use an id"
        );
        assert_eq!(
            shared.data,
            Some(serde_json::json!([one.to_string(), two.to_string()])),
            "the candidates are the ids, so the caller can name one"
        );
    }

    #[test]
    fn naming_a_workspace_the_name_it_already_has_reports_nothing() {
        let (mut m, pid, _) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        m.rename_workspace(&w1, Some("auth cleanup".into()))
            .unwrap();
        assert_eq!(
            m.rename_workspace(&w1, Some(" auth cleanup ".into()))
                .unwrap(),
            Vec::<Event>::new(),
            "a name that did not change is not something to tell subscribers about"
        );
        assert_eq!(
            m.rename_workspace(&w1, None).unwrap(),
            vec![Event::WorkspaceRenamed {
                workspace: w1.clone(),
                name: None
            }]
        );
        assert_eq!(
            m.rename_workspace(&w1, None).unwrap(),
            Vec::<Event>::new(),
            "and clearing a name that is already clear is not either"
        );
        assert_eq!(
            m.rename_workspace(&WorkspaceId("w_0000".into()), None)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn pruning_a_workspace_says_the_path_went_away_rather_than_that_someone_deleted_it() {
        let (mut m, pid, main) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        assert_eq!(
            m.prune_workspace(&main).unwrap_err().code,
            ErrorCode::Refused,
            "main is refused on this path too"
        );
        let (handle, events) = m.prune_workspace(&w1).unwrap();
        assert_eq!(handle, WorkspaceHandle::Slot(1));
        assert_eq!(
            events,
            vec![Event::WorkspaceDeleted {
                project: pid,
                workspace: w1.clone(),
                handle: "workspace-1".into(),
                pruned: true
            }]
        );
        assert!(m.workspace(&w1).is_none());
        assert!(m.retired.iter().any(|r| r == w1.as_str()));
    }

    #[test]
    fn a_branch_repeats_line_one_only_when_it_equals_the_handle() {
        let (mut m, pid, main) = git_model();
        let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
        let w = m.workspace(&w1).unwrap();
        assert!(w.branch_is_handle("workspace-1"));
        assert!(!w.branch_is_handle("feat/x"));
        assert!(
            !w.branch_is_handle("main"),
            "that is another workspace's handle"
        );
        assert!(m.workspace(&main).unwrap().branch_is_handle("main"));
        m.rename_workspace(&w1, Some("auth cleanup".into()))
            .unwrap();
        assert!(
            m.workspace(&w1).unwrap().branch_is_handle("workspace-1"),
            "a name does not change which branch the branch line would repeat"
        );
    }

    #[test]
    fn closing_an_overlay_clears_what_was_being_typed_into_it() {
        let (_m, ws, tab, pane) = model_with_one_tab();
        let mut view = client("c_0001", &ws, &tab, &pane);
        view.push_overlay(Overlay::Switcher);
        view.filtering = true;
        view.push_overlay(Overlay::NameWorkspace(ws.clone()));
        view.input = TextInput::new("auth cleanup");
        view.pop_overlay();
        assert_eq!(
            view.input,
            TextInput::new(""),
            "a half-typed name must not turn up inside the overlay underneath"
        );
        assert!(!view.filtering, "and neither must a half-typed filter");
    }
}
