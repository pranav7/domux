//! The Model: the root of all shared state, owned by the core task. Every mutation is a
//! method here that returns the events it produced. Nothing outside this crate writes fields.

pub mod agent;
pub mod focus;
pub mod layout;

pub use agent::{transition, Agent, AgentEvent, AgentKind, AgentReport, AgentSource, AgentState};
pub use focus::{ConfirmKind, Focus, Overlay, PromptKind, RegionKind, RowTarget, TextInput};
pub use layout::{Direction, LayoutNode, Pane, PaneContent, Rect, SplitDir};

use crate::api::{ApiError, Event};
use crate::ids::{AgentId, ClientId, IdGen, PaneId, ProjectId, TabId, WorkspaceId};
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
    /// M3: one record per AI coding session. Persisted; live ones come back as exited.
    #[serde(default)]
    pub agents: Vec<Agent>,
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
    /// Whether domux is holding this machine awake. Persisted for the reason the sidebar is:
    /// a switch the reader flipped is about the next few hours, not about this run of the
    /// server. The hold itself is a child process the server owns; this says whether there
    /// should be one, and the server keeps the two in step (decision 0029).
    #[serde(default)]
    pub stay_awake: bool,
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
            agents,
            clients,
            last_workspace,
            sidebar_open,
            stay_awake,
            idgen: _,
            activity_seq: _,
            retired: _,
        } = self;
        *projects == other.projects
            && *agents == other.agents
            && *clients == other.clients
            && *last_workspace == other.last_workspace
            && *sidebar_open == other.sidebar_open
            && *stay_awake == other.stay_awake
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

/// The refusal `main` earns, in one spelling. `Model::remove_workspace` is the last guard
/// and `api::workspace::delete` is the first, and a reader who met one and then the other
/// must not be told two different things (principle 10).
pub const MAIN_CANNOT_BE_DELETED: &str =
    "main is the project's checkout and cannot be deleted; delete a workspace-N slot instead";

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

impl WorkspaceHandle {
    /// True when `text` reads as a handle: `main`, or `workspace-` and a number, in any case
    /// and ignoring surrounding space. That is exactly what the handle pass of
    /// `Model::resolve_workspace_with` can match, so a name that answers true here could
    /// never resolve to the workspace it was given to: the handle pass runs first and returns
    /// the workspace whose handle it is.
    ///
    /// The grammar, not the handles a model happens to hold. A name checked against today's
    /// handles would be legal until someone made that slot, and nothing would look again.
    ///
    /// The round trip through `Display` is what settles the edge cases: `workspace-01` parses
    /// as a number but no handle prints it, so nothing could match it and it is a perfectly
    /// good name.
    pub fn reads_as_handle(text: &str) -> bool {
        let text = text.trim().to_ascii_lowercase();
        if text == WorkspaceHandle::Main.to_string() {
            return true;
        }
        text.strip_prefix("workspace-")
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| WorkspaceHandle::Slot(n).to_string() == text)
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
    /// M3: the row the keys act on in an Agents box, as the agent under it, so the cursor
    /// survives a re-sort (principle 2).
    #[serde(default)]
    pub agents_cursor: Option<AgentId>,
    /// The first visible line inside an Agents box, beside `agents_cursor` as
    /// `projects_scroll` is beside `projects_cursor`, so scrolling moves as little as it can
    /// when the cursor leaves the view.
    #[serde(default)]
    pub agents_scroll: u16,
    /// The row the keys act on in the Navigator, which lists workspaces and the agents
    /// running in them, so its cursor holds either (decision record 0030).
    ///
    /// Its own field rather than a widened `projects_cursor`, because the two boxes the
    /// `[navigator]` key turns back on keep their own cursors until they are deleted.
    #[serde(default)]
    pub navigator_cursor: Option<RowTarget>,
    /// The first visible line inside the Navigator, as `projects_scroll` is beside
    /// `projects_cursor`.
    #[serde(default)]
    pub navigator_scroll: u16,
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

    /// Where the keys go once `pop_overlay` has run: the overlay it uncovered, or `fallback`
    /// when it uncovered nothing. Never a frame with the keys in a region nothing on the
    /// screen marks (principle 2).
    ///
    /// The switcher and the agents overlay are named rather than lumped in with `Overlay`,
    /// because the region is what says which key table the reader is holding and each of those
    /// two holds a box with one of its own. `render::overlay::draw_help` reads exactly that to
    /// decide which table to list first, so an uncovered box left as `Overlay` would answer a
    /// reader standing in it with a help screen missing the keys they are holding.
    /// Answered here rather than at each of the three callers - `api::focus::pane`,
    /// `api::switcher::close` and `input::close_overlay` - which wrote the same match out
    /// three times.
    ///
    /// The fallback is the caller's because the three ask two different questions, and only
    /// where nothing is left underneath. Closing an overlay gives the keys back to whatever
    /// had them, so `?` in a box comes back to the box: `focus_returning_from_overlay`.
    /// `focus.pane` is a request to leave, and Esc in the sidebar's box returns to the pane
    /// you left (interface spec 5.4), so it passes `focus_on_pane`. The difference is one
    /// case, and it is spelled at the call rather than guessed here.
    pub fn focus_after_pop(&self, fallback: Focus) -> Focus {
        match &self.overlay {
            Some(Overlay::Switcher) => Focus::Region(RegionKind::Switcher),
            Some(Overlay::Agents) => Focus::Region(RegionKind::AgentsOverlay),
            Some(_) => Focus::Region(RegionKind::Overlay),
            None => fallback,
        }
    }

    /// The pane, or the focus this view already has when it has no pane to go to.
    pub fn focus_on_pane(&self, pane: Option<PaneId>) -> Focus {
        match pane {
            Some(p) => Focus::Pane(p),
            None => self.focus.clone(),
        }
    }

    /// Where the keys go when an overlay closes over no other: back to the box that had them
    /// while it was open, and to the pane when there is no such box.
    ///
    /// `api::client::help` keeps a box's region while the help is over it, so this is what
    /// makes `?` in the sidebar's Projects box come back to the box, the same way `?` over the
    /// switcher comes back to the switcher. Half of that rule would be worse than either
    /// whole: a reader who learns one surface would be surprised by the other.
    ///
    /// `sidebar_visible` earns its place. A client narrowed below `SIDEBAR_MIN_COLS` while the
    /// help was open has no box left to come back to, and the keys would land in a region
    /// nothing on the screen marks (principle 2).
    ///
    /// The sidebar's two boxes and no other: the switcher's box and the agents overlay's box
    /// are overlays and `focus_after_pop` answers for them above, so what is left is what the
    /// sidebar draws under an overlay and still owns once it closes.
    pub fn focus_returning_from_overlay(&self, pane: Option<PaneId>) -> Focus {
        if matches!(self.focus, Focus::Region(kind) if kind.is_sidebar()) && self.sidebar_visible()
        {
            return self.focus.clone();
        }
        self.focus_on_pane(pane)
    }
}

/// A one-line result in the hint row or the footer: green when it worked, red when it was
/// refused (interface spec 7.3).
///
/// Interface spec 12.12 says it clears on the next key in a box or after `PILL_SECONDS`.
/// Only the second half is built: `Core::expire_pills` drops a pill on the tick that takes it
/// past `PILL_SECONDS`, and no key clears one. `Core.notes`, which shares these two rows and
/// is described as clearing "like a pill", is the other way round - `Core::clear_notes_read_by`
/// clears it on the first key in a box and nothing ages it out. So the two behave differently
/// today despite reading as one rule, and this comment says which is which rather than
/// describing the rule neither of them fully implements.
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
            agents: Vec::new(),
            clients: Vec::new(),
            last_workspace: None,
            sidebar_open: false,
            stay_awake: false,
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
            "a" => "no free agent id: every draw hit an id already in use or recently closed, so close an agent or two",
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
            || self.agents.iter().any(|a| a.id.as_str() == id)
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
    ///
    /// Focusing the agent's pane clears its dot (interface spec 6.5), and that happens here
    /// rather than in each caller, because focus reaches a pane through several routes and
    /// they all end up here.
    ///
    /// **The clear runs even when the pane was already focused.** Whether the focused pane
    /// moved is a different question, and the early return still answers it: a `PaneFocused`
    /// event for a pane that did not move would tell a subscriber something that did not
    /// happen. The dot is not about movement. Every caller of this method is a reader act - a
    /// directional move, `focus.last`, `pane.focus`, `pane.zoom`, a scroll gesture - and none
    /// of them runs on a timer, so a call naming the pane you are already on is still someone
    /// asking for that pane. Without this, a dot that arrived while you sat in the pane would
    /// survive a deliberate focus of it and clear only on the next keystroke. Nothing about
    /// 6.5's other half is weakened: a dot nobody acted on still survives, because nothing
    /// calls this without a reader asking.
    pub fn focus_pane(&mut self, pane: &PaneId) -> Result<Vec<Event>, ApiError> {
        let loc = self.pane_location(pane).ok_or_else(|| {
            ApiError::not_found(format!(
                "pane {pane} does not exist; run {BIN_NAME} api pane.list"
            ))
        })?;
        let t = self.tab_mut(&loc.tab).expect("tab exists");
        let mut events = Vec::new();
        if &t.focused != pane {
            t.last_focused = Some(t.focused.clone());
            t.focused = pane.clone();
            let tab_id = loc.tab.clone();
            for c in &mut self.clients {
                if c.tab == tab_id && matches!(c.focus, Focus::Pane(_)) {
                    c.focus = Focus::Pane(pane.clone());
                }
            }
            events.push(Event::PaneFocused {
                tab: loc.tab,
                pane: pane.clone(),
            });
        }
        events.extend(self.clear_unseen_for_pane(pane));
        Ok(events)
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
            return Err(ApiError::refused(MAIN_CANNOT_BE_DELETED));
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
        // Every record of this workspace goes with it (M3 plan assumption 32). Here rather
        // than in the handler, because a record naming a workspace the model does not hold
        // is not a state anything can draw: it has no place line, nothing can focus it and
        // nothing can resume it. This method and `remove_project` are the only two ways a
        // workspace leaves the model, so between them the rule cannot be forgotten by a
        // later caller.
        let mut events = self.remove_agents_of_workspace(id);
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
        events.push(Event::WorkspaceDeleted {
            project,
            workspace: id.clone(),
            handle: handle.to_string(),
            pruned,
        });
        Ok((handle, events))
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
        // The records of every workspace that went, for the reason `remove_workspace_inner`
        // gives: this is the other way a workspace leaves the model.
        let mut events = Vec::new();
        for w in &gone {
            events.extend(self.remove_agents_of_workspace(w));
        }
        events.push(Event::ProjectRemoved {
            project: id.clone(),
            name,
        });
        Ok(events)
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

/// What `report_agent` did: the record the payload landed on, the states it moved between,
/// whether the report made the record, and the events to publish. `report_agent` answers
/// `None` when the payload landed on no record at all.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentReportOutcome {
    pub agent: AgentId,
    pub from: AgentState,
    /// `None` when the report ended the session, in which case the record has been removed
    /// and `agent` names an id nothing holds any more (decision record 0030).
    pub to: Option<AgentState>,
    pub created: bool,
    pub events: Vec<Event>,
}

/// The agent records (M3). Every mutation answers with the events it produced, as the rest
/// of the Model does.
impl Model {
    pub fn agent(&self, id: &AgentId) -> Option<&Agent> {
        self.agents.iter().find(|a| &a.id == id)
    }

    pub fn agent_mut(&mut self, id: &AgentId) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| &a.id == id)
    }

    /// The one agent on a pane, if any. Every record is live, so a pane holds at most one.
    pub fn agent_on_pane(&self, pane: &PaneId) -> Option<&Agent> {
        self.agents.iter().find(|a| a.pane.as_ref() == Some(pane))
    }

    /// Every record of a workspace, in creation order.
    pub fn agents_in_workspace(&self, ws: &WorkspaceId) -> Vec<&Agent> {
        self.agents.iter().filter(|a| &a.workspace == ws).collect()
    }

    /// Applies one hook payload from `pane`.
    ///
    /// Finds the record by session id, else the record of that kind on the pane (the
    /// observer's placeholder), else creates one. A second session id on a pane ends the first
    /// record, because one pane hosts at most one agent.
    pub fn report_agent(
        &mut self,
        pane: &PaneId,
        kind: AgentKind,
        report: AgentReport,
        now: &str,
    ) -> Result<Option<AgentReportOutcome>, ApiError> {
        let loc = self.pane_location(pane).ok_or_else(|| {
            ApiError::not_found(format!(
                "pane {pane} does not exist; the hook ran outside a domux pane or the pane closed"
            ))
        })?;
        let event = report
            .event
            .ok_or_else(|| ApiError::invalid_params("the payload names no event domux tracks"))?;
        let mut events = Vec::new();
        let by_session = report.session_id.as_deref().and_then(|sid| {
            self.agents
                .iter()
                .position(|a| a.kind == kind && a.session_id.as_deref() == Some(sid))
        });
        let live_here = self
            .agents
            .iter()
            .position(|a| a.pane.as_ref() == Some(pane));
        let (index, created) = match (by_session, live_here) {
            (Some(i), Some(j)) if i != j => {
                if self.agents[j].session_id.is_none() {
                    // A placeholder the observer made for the session that is resuming. It
                    // never had a session id of its own, so nothing is lost by dropping it.
                    let gone = self.agents.remove(j);
                    // Retired like every other removed id; `remove_agent` says why.
                    self.retire(gone.id.to_string());
                    events.push(Event::AgentExited {
                        agent: gone.id,
                        pane: None,
                    });
                    (if j < i { i - 1 } else { i }, false)
                } else {
                    events.extend(self.end_record(j));
                    // `end_record` removed row `j`, so a row after it has moved up one.
                    (if j < i { i - 1 } else { i }, false)
                }
            }
            (Some(i), _) => (i, false),
            (None, Some(j))
                if self.agents[j].kind == kind && self.agents[j].session_id.is_none() =>
            {
                (j, false)
            }
            (None, Some(j)) => {
                // A different session took the pane: the old record ends, a new one starts.
                events.extend(self.end_record(j));
                (
                    self.new_agent(kind, &loc, pane, AgentSource::Hook, now)?,
                    true,
                )
            }
            // Nothing on this pane and nothing with this session id. Only `SessionStart`
            // makes a record: every other hook is a message from a session domux is not
            // tracking, and inventing a record for one is how a hook arriving after
            // `SessionEnd` would leave a row behind that nothing could take away (decision
            // record 0030). A session domux missed the start of still gets a record, from the
            // observer, the moment its process is in front of a pane.
            (None, None) if event != AgentEvent::SessionStart => return Ok(None),
            (None, None) => (
                self.new_agent(kind, &loc, pane, AgentSource::Hook, now)?,
                true,
            ),
        };
        let pane_cwd = self.pane(pane).map(|p| p.cwd.clone());
        let a = &mut self.agents[index];
        let from = a.state;
        let to = transition(from, event);
        if created {
            events.push(Event::AgentCreated {
                agent: a.id.clone(),
                kind,
                pane: Some(pane.clone()),
                source: AgentSource::Hook,
            });
        }
        // `SessionEnd` from any state. The session is over, so the record goes with it and
        // nothing below runs: there is no row left to write a recap or a session name onto
        // (decision record 0030).
        if to.is_none() {
            let id = a.id.clone();
            events.extend(self.end_record(index));
            return Ok(Some(AgentReportOutcome {
                agent: id,
                from,
                to: None,
                created,
                events,
            }));
        }
        let to = to.expect("the ending case returned above");
        if a.pane.as_ref() != Some(pane) {
            a.pane = Some(pane.clone());
            a.last_pane = Some(pane.clone());
            a.workspace = loc.workspace.clone();
        }
        if let Some(sid) = report.session_id {
            a.session_id = Some(sid);
        }
        if let Some(path) = report.transcript_path {
            a.transcript_path = Some(path);
        }
        if event == AgentEvent::SessionStart {
            if let Some(cwd) = report.cwd.or(pane_cwd) {
                a.cwd = cwd;
            }
        }
        if event == AgentEvent::Notification {
            a.reason = report.reason;
        } else if to != AgentState::Waiting {
            // The reason belongs to the waiting state, so it goes when the state does.
            a.reason = None;
        }
        a.last_activity_at = now.to_string();
        a.state = to;
        let id = a.id.clone();
        if from != to {
            events.push(Event::AgentStateChanged {
                agent: id.clone(),
                from,
                to,
            });
        }
        if agent::attention(from, to) && !a.unseen {
            a.unseen = true;
            events.push(Event::AgentUnseenChanged {
                agent: id.clone(),
                unseen: true,
            });
        }
        Ok(Some(AgentReportOutcome {
            agent: id,
            from,
            to: Some(to),
            created,
            events,
        }))
    }

    /// Adds an `unknown` record on `pane` and answers with its index. The caller applies the
    /// first event.
    ///
    /// Fallible for one reason: `next_id` is.
    fn new_agent(
        &mut self,
        kind: AgentKind,
        loc: &PaneLocation,
        pane: &PaneId,
        source: AgentSource,
        now: &str,
    ) -> Result<usize, ApiError> {
        let id = AgentId(self.next_id("a")?);
        let cwd = self.pane(pane).map(|p| p.cwd.clone()).unwrap_or_default();
        self.agents.push(Agent::new(
            id,
            kind,
            loc.workspace.clone(),
            pane.clone(),
            cwd,
            source,
            now,
        ));
        Ok(self.agents.len() - 1)
    }

    /// Ends the record at `index` and removes it, which is what `SessionEnd` and
    /// `ProcessGone` mean (decision record 0030). The pane it was on travels in the event,
    /// because the record is gone by the time anyone reads it.
    ///
    /// Retires the id for the reason `remove_agent` gives.
    fn end_record(&mut self, index: usize) -> Vec<Event> {
        let a = &mut self.agents[index];
        let pane = a.pane.take();
        let id = a.id.clone();
        self.agents.remove(index);
        self.retire(id.to_string());
        vec![Event::AgentExited { agent: id, pane }]
    }

    /// The observer saw a known agent command on `pane` with no live record there.
    ///
    /// Infallible, unlike `report_agent`: the observer runs on the core's tick and has no
    /// caller to answer. It names only panes the model holds, and `next_id` refuses only
    /// when every draw of an agent id collides, which takes an id space full of agents.
    /// Both are model bugs rather than states the observer can be in.
    pub fn observe_agent(
        &mut self,
        pane: &PaneId,
        kind: AgentKind,
        pid: Option<u32>,
        now: &str,
    ) -> (AgentId, Vec<Event>) {
        let loc = self
            .pane_location(pane)
            .expect("the observer only names panes in the model");
        let index = self
            .new_agent(kind, &loc, pane, AgentSource::Observer, now)
            .expect("an agent id is free");
        self.agents[index].pid = pid;
        let id = self.agents[index].id.clone();
        (
            id.clone(),
            vec![Event::AgentCreated {
                agent: id,
                kind,
                pane: Some(pane.clone()),
                source: AgentSource::Observer,
            }],
        )
    }

    /// The observer saw the process leave, or the pane exited or closed. The record goes with
    /// the session (decision record 0030).
    pub fn agent_process_gone(&mut self, id: &AgentId) -> Vec<Event> {
        self.remove_agent(id)
    }

    pub fn set_agent_pid(&mut self, id: &AgentId, pid: Option<u32>) {
        if let Some(a) = self.agent_mut(id) {
            a.pid = pid;
        }
    }

    /// A recap that did not change produces no event.
    pub fn set_agent_recap(&mut self, id: &AgentId, recap: Option<String>) -> Vec<Event> {
        match self.agent_mut(id) {
            Some(a) if a.recap != recap => {
                a.recap = recap.clone();
                vec![Event::AgentRecapChanged {
                    agent: id.clone(),
                    recap,
                }]
            }
            _ => Vec::new(),
        }
    }

    /// Sets or clears the session name, and answers with whether it changed. `Some("")` and
    /// whitespace clear, as `rename_tab` does. No event: the name is not a transition.
    pub fn set_agent_name(&mut self, id: &AgentId, name: Option<String>) -> bool {
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        match self.agent_mut(id) {
            Some(a) if a.name != name => {
                a.name = name;
                true
            }
            _ => false,
        }
    }

    /// Removes a record and retires its id.
    ///
    /// Every removal path in this file retires the id. A client's cursor holds an agent id,
    /// and so do the server's per-agent working word and recap, so an id handed back to a new
    /// session would show a dead session's recap under a live agent.
    ///
    /// No verb reaches this. `agent.dismiss` was the one, and decision record 0030 removed it
    /// along with the exited record it was there to tidy away.
    fn remove_agent(&mut self, id: &AgentId) -> Vec<Event> {
        let Some(index) = self.agents.iter().position(|a| &a.id == id) else {
            return Vec::new();
        };
        self.end_record(index)
    }

    pub fn clear_unseen(&mut self, id: &AgentId) -> Vec<Event> {
        match self.agent_mut(id) {
            Some(a) if a.unseen => {
                a.unseen = false;
                vec![Event::AgentUnseenChanged {
                    agent: id.clone(),
                    unseen: false,
                }]
            }
            _ => Vec::new(),
        }
    }

    /// Focusing a pane or typing into it clears unseen on the agent there.
    pub fn clear_unseen_for_pane(&mut self, pane: &PaneId) -> Vec<Event> {
        let ids: Vec<AgentId> = self
            .agents
            .iter()
            .filter(|a| a.unseen && a.pane.as_ref() == Some(pane))
            .map(|a| a.id.clone())
            .collect();
        ids.iter().flat_map(|id| self.clear_unseen(id)).collect()
    }

    /// Clearing or deleting a workspace removes its records (architecture spec 3.2).
    pub fn remove_agents_of_workspace(&mut self, ws: &WorkspaceId) -> Vec<Event> {
        let ids: Vec<AgentId> = self
            .agents
            .iter()
            .filter(|a| &a.workspace == ws)
            .map(|a| a.id.clone())
            .collect();
        ids.iter().flat_map(|id| self.remove_agent(id)).collect()
    }

    /// Interface spec 6.7 and 12.28: waiting first, then working and compacting by last
    /// activity, then unseen idle, then quiet idle, then unknown, each group newest first.
    /// The architecture spec's 3.7 puts unseen idle ahead of working and says the plans take
    /// this order; `rank` is the whole difference between the two.
    ///
    /// The Navigator does not use this. It lists agents under the workspace they run in, in
    /// the order they started, so that no row moves while a state changes (decision record
    /// 0030). This order is for `agent.list` and `peek`, which are read as lists of agents.
    pub fn sorted_agents(&self) -> Vec<&Agent> {
        fn rank(a: &Agent) -> u8 {
            match (a.state, a.unseen) {
                (AgentState::Waiting, _) => 0,
                (AgentState::Working | AgentState::Compacting, _) => 1,
                (AgentState::Idle, true) => 2,
                (AgentState::Idle, false) => 3,
                (AgentState::Unknown, _) => 4,
            }
        }
        let mut agents: Vec<&Agent> = self.agents.iter().collect();
        // The id breaks a tie, so two records that changed in the same second keep one order
        // between renders rather than swapping places.
        agents.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then_with(|| b.last_activity_at.cmp(&a.last_activity_at))
                .then_with(|| a.id.cmp(&b.id))
        });
        agents
    }

    /// Agents with a red dot across every project: the number `agent.list` answers with.
    ///
    /// It was the top bar's count too until MUX-23 took that off the bar. Nothing on a screen
    /// reads it now; a caller with a command line still does.
    pub fn red_dot_count(&self) -> usize {
        self.agents.iter().filter(|a| a.needs_you()).count()
    }

    /// An agent id; or a workspace (id, handle, name or branch) holding exactly one record;
    /// or `workspace/tab` when it holds more (architecture spec section 7).
    ///
    /// It took the calling verb's precondition until decision record 0030, which left every
    /// record live and every verb able to act on any of them.
    pub fn resolve_agent_target(&self, target: &str) -> Result<AgentId, ApiError> {
        if let Ok(id) = target.parse::<AgentId>() {
            return self.agent(&id).map(|a| a.id.clone()).ok_or_else(|| {
                ApiError::not_found(format!(
                    "agent {target} does not exist; run {BIN_NAME} peek"
                ))
            });
        }
        let (ws, tab) = match self.resolve_workspace(target) {
            Ok(ws) => (ws, None),
            // The workspace error is the one to report: `workspace/tab` is the qualified
            // form, so a target that is neither failed as a workspace name.
            Err(first) => match target.rsplit_once('/') {
                Some((w, t)) => (self.resolve_workspace(w).map_err(|_| first)?, Some(t)),
                None => return Err(first),
            },
        };
        let mut found: Vec<&Agent> = self.agents.iter().filter(|a| a.workspace == ws).collect();
        let tab_of = |a: &Agent| {
            a.pane
                .as_ref()
                .and_then(|p| self.pane_location(p))
                .map(|l| l.tab)
        };
        if let Some(t) = tab {
            let tab_id = self.resolve_tab(&ws, t)?;
            found.retain(|a| tab_of(a).is_some_and(|id| id == tab_id));
        }
        match found.len() {
            0 => Err(ApiError::not_found(format!(
                "no agent in {target}; run {BIN_NAME} peek"
            ))),
            1 => Ok(found[0].id.clone()),
            n => {
                let tabs: Vec<Option<String>> = found
                    .iter()
                    .map(|a| {
                        tab_of(a)
                            .and_then(|id| self.tab(&id))
                            .map(|t| t.name.clone().unwrap_or_else(|| t.id.to_string()))
                    })
                    .collect();
                let candidates = found
                    .iter()
                    .zip(&tabs)
                    .map(|(a, tab)| {
                        format!(
                            "{} {}{}",
                            a.id,
                            a.kind,
                            tab.as_ref()
                                .map(|t| format!(" in tab {t}"))
                                .unwrap_or_default()
                        )
                    })
                    .collect();
                // The example names a tab one of these agents is in, so retyping it
                // works. A target that already names a tab has no further qualifier to
                // offer, and appending a second one would read "main/pr1/pr1", which
                // resolves to nothing; there the message asks for an id instead.
                let example = if tab.is_some() {
                    None
                } else {
                    tabs.iter().flatten().next()
                };
                let message = match example {
                    Some(t) => format!(
                        "{n} agents are in {target}; qualify with the tab, for \
                         example \"{target}/{t}\", or use an agent id"
                    ),
                    None => format!("{n} agents are in {target}; use an agent id"),
                };
                Err(ApiError::ambiguous(message, candidates))
            }
        }
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
            agents_cursor: None,
            agents_scroll: 0,
            navigator_cursor: None,
            navigator_scroll: 0,
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

    /// Where the keys go after a pop, for every shape of stack. `Switcher` and `AgentsOverlay`
    /// rather than `Overlay` for the two overlays that hold a box, because the region is what
    /// says which key table the reader is holding and `render::overlay::draw_help` reads it to
    /// decide which table to list first.
    ///
    /// The `NameWorkspace` case is the one that separates those arms from the last: every one
    /// of the three leaves an overlay open, and an implementation answering `Overlay` for all
    /// of them would pass a fixture that only ever uncovered a modal.
    #[test]
    fn the_keys_go_to_what_a_pop_uncovers_and_a_box_that_comes_back_is_named_as_one() {
        let mut m = Model::new(7);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let (tab, pane, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let mut view = client("c_0001", &ws, &tab, &pane);

        view.push_overlay(Overlay::Switcher);
        view.push_overlay(Overlay::Help);
        view.pop_overlay();
        assert_eq!(
            view.focus_after_pop(view.focus_on_pane(Some(pane.clone()))),
            Focus::Region(RegionKind::Switcher),
            "a switcher that comes back is a box, not any old modal"
        );

        view.push_overlay(Overlay::Agents);
        view.push_overlay(Overlay::Help);
        view.pop_overlay();
        assert_eq!(
            view.focus_after_pop(view.focus_on_pane(Some(pane.clone()))),
            Focus::Region(RegionKind::AgentsOverlay),
            "and so is an agents overlay that comes back"
        );

        view.push_overlay(Overlay::NameWorkspace(ws.clone()));
        view.push_overlay(Overlay::Help);
        view.pop_overlay();
        assert_eq!(
            view.focus_after_pop(view.focus_on_pane(Some(pane.clone()))),
            Focus::Region(RegionKind::Overlay),
            "a name box has no key table of its own"
        );

        view.pop_overlay();
        view.pop_overlay();
        assert_eq!(view.overlay, None);
        assert_eq!(
            view.focus_after_pop(view.focus_on_pane(Some(pane.clone()))),
            Focus::Pane(pane.clone()),
            "with nothing left the keys go back to the pane"
        );
        let before = view.focus.clone();
        assert_eq!(
            view.focus_after_pop(view.focus_on_pane(None)),
            before,
            "and a client with no pane keeps the focus it had rather than losing it"
        );
    }

    /// A box is a region with its own `[keys.list]` table; `Overlay` is every modal that has
    /// none. Derived over every variant, so a variant added later is not silently a box.
    #[test]
    fn every_region_but_overlay_is_a_box() {
        for kind in [
            RegionKind::Switcher,
            RegionKind::AgentsOverlay,
            RegionKind::SidebarProjects,
            RegionKind::SidebarAgents,
        ] {
            assert!(kind.is_box(), "{kind:?} holds a list of its own");
        }
        assert!(!RegionKind::Overlay.is_box());
    }

    /// The two boxes in the sidebar's column and no others. The match inside is the forcing
    /// function: a region kind added later makes it non-exhaustive, so whoever adds one has to
    /// come here and say which side of the line it is on. The list above it is written out, so
    /// they have to add the kind there too for the assertion to reach it.
    #[test]
    fn only_the_two_boxes_in_the_column_are_sidebar_regions() {
        for kind in [
            RegionKind::Switcher,
            RegionKind::AgentsOverlay,
            RegionKind::SidebarProjects,
            RegionKind::SidebarAgents,
            RegionKind::Overlay,
        ] {
            let in_the_column = match kind {
                RegionKind::SidebarProjects | RegionKind::SidebarAgents => true,
                RegionKind::Switcher | RegionKind::AgentsOverlay | RegionKind::Overlay => false,
            };
            assert_eq!(kind.is_sidebar(), in_the_column, "{kind:?}");
        }
    }

    /// The cases are derived from `Display` rather than written out beside it, so a change to
    /// how a handle prints cannot leave this test agreeing with the old spelling.
    ///
    /// `workspace-01` and `workspace-+1` are the interesting refusals: both parse as the
    /// number one, and neither is a handle, because no handle prints them and so the handle
    /// pass of `resolve_workspace_with` could never match them. Someone may name a workspace
    /// either.
    #[test]
    fn a_string_reads_as_a_handle_exactly_when_some_handle_prints_it() {
        for handle in [
            WorkspaceHandle::Main,
            WorkspaceHandle::Slot(1),
            WorkspaceHandle::Slot(42),
        ] {
            let printed = handle.to_string();
            assert!(WorkspaceHandle::reads_as_handle(&printed), "{printed}");
            assert!(
                WorkspaceHandle::reads_as_handle(&printed.to_uppercase()),
                "the handle pass ignores case, so this must too: {printed}"
            );
            assert!(
                WorkspaceHandle::reads_as_handle(&format!("  {printed} ")),
                "the resolver trims its target, so this must too: {printed}"
            );
        }
        for name in [
            "auth cleanup",
            "workspace",
            "workspace-",
            "workspace-01",
            "workspace-+1",
            "workspace-x",
            "workspace-1a",
            "mainline",
            "",
        ] {
            assert!(!WorkspaceHandle::reads_as_handle(name), "{name}");
        }
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

    use crate::model::agent::{AgentEvent, AgentKind, AgentReport, AgentSource, AgentState};

    const T0: &str = "2026-09-04T14:32:00+00:00";
    const T1: &str = "2026-09-04T14:33:00+00:00";

    fn hook(event: AgentEvent, sid: &str) -> AgentReport {
        AgentReport {
            event: Some(event),
            session_id: Some(sid.into()),
            transcript_path: Some(PathBuf::from(format!("/t/{sid}.jsonl"))),
            cwd: Some(PathBuf::from("/Users/pranav/projects/domux")),
            reason: None,
        }
    }

    #[test]
    fn a_session_start_on_a_fresh_pane_creates_an_idle_record_bound_to_the_pane() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        let out = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "sid-1"),
                T0,
            )
            .unwrap()
            .unwrap();
        assert!(out.created);
        assert_eq!(
            (out.from, out.to),
            (AgentState::Unknown, Some(AgentState::Idle))
        );
        let a = m.agent(&out.agent).unwrap();
        assert_eq!(a.session_id.as_deref(), Some("sid-1"));
        assert_eq!(a.pane, Some(pane.clone()));
        assert_eq!(a.last_pane, Some(pane.clone()));
        assert_eq!(a.workspace, ws);
        assert_eq!(a.cwd, PathBuf::from("/Users/pranav/projects/domux"));
        assert_eq!(a.transcript_path, Some(PathBuf::from("/t/sid-1.jsonl")));
        assert_eq!(a.source, AgentSource::Hook);
        assert!(!a.unseen, "starting is not an attention transition");
        assert_eq!(
            out.events[0],
            Event::AgentCreated {
                agent: out.agent.clone(),
                kind: AgentKind::Claude,
                pane: Some(pane.clone()),
                source: AgentSource::Hook
            }
        );
        assert_eq!(
            out.events[1],
            Event::AgentStateChanged {
                agent: out.agent.clone(),
                from: AgentState::Unknown,
                to: AgentState::Idle
            }
        );
        assert_eq!(out.events.len(), 2);
        assert_eq!(m.pane_location(&pane).unwrap().tab, tab);
    }

    #[test]
    fn a_hook_binds_the_observers_unknown_record_instead_of_making_a_second_one() {
        let (mut m, _, _, pane) = model_with_one_tab();
        let (id, events) = m.observe_agent(&pane, AgentKind::Claude, Some(4242), T0);
        assert_eq!(
            events,
            vec![Event::AgentCreated {
                agent: id.clone(),
                kind: AgentKind::Claude,
                pane: Some(pane.clone()),
                source: AgentSource::Observer
            }]
        );
        assert_eq!(m.agent(&id).unwrap().state, AgentState::Unknown);
        let out = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::UserPromptSubmit, "sid-2"),
                T1,
            )
            .unwrap()
            .unwrap();
        assert_eq!(out.agent, id);
        assert!(!out.created);
        assert_eq!(m.agents.len(), 1);
        assert_eq!(m.agent(&id).unwrap().session_id.as_deref(), Some("sid-2"));
        assert_eq!(m.agent(&id).unwrap().state, AgentState::Working);
        assert_eq!(m.agent(&id).unwrap().pid, Some(4242));
    }

    #[test]
    fn waiting_working_to_idle_and_exit_set_unseen_and_report_it_once() {
        let (mut m, _, _, pane) = model_with_one_tab();
        let id = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        m.report_agent(
            &pane,
            AgentKind::Claude,
            hook(AgentEvent::UserPromptSubmit, "s"),
            T0,
        )
        .unwrap()
        .unwrap();
        let mut waiting = hook(AgentEvent::Notification, "s");
        waiting.reason = Some("Claude needs your permission to use Bash".into());
        let out = m
            .report_agent(&pane, AgentKind::Claude, waiting, T1)
            .unwrap()
            .unwrap();
        assert!(out.events.contains(&Event::AgentUnseenChanged {
            agent: id.clone(),
            unseen: true
        }));
        assert_eq!(
            m.agent(&id).unwrap().reason.as_deref(),
            Some("Claude needs your permission to use Bash")
        );
        let out = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::PreToolUse, "s"),
                T1,
            )
            .unwrap()
            .unwrap();
        assert!(
            !out.events
                .iter()
                .any(|e| matches!(e, Event::AgentUnseenChanged { .. })),
            "still unseen; no second event"
        );
        assert!(
            m.agent(&id).unwrap().unseen,
            "leaving waiting does not clear unseen"
        );
        assert_eq!(
            m.agent(&id).unwrap().reason,
            None,
            "the reason goes with the waiting state"
        );
        assert_eq!(
            m.clear_unseen(&id),
            vec![Event::AgentUnseenChanged {
                agent: id.clone(),
                unseen: false
            }]
        );
        let out = m
            .report_agent(&pane, AgentKind::Claude, hook(AgentEvent::Stop, "s"), T1)
            .unwrap()
            .unwrap();
        assert_eq!(
            (out.from, out.to),
            (AgentState::Working, Some(AgentState::Idle))
        );
        assert!(m.agent(&id).unwrap().unseen, "working to idle");
        m.clear_unseen_for_pane(&pane);
        let out = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionEnd, "s"),
                T1,
            )
            .unwrap()
            .unwrap();
        assert_eq!(out.to, None, "the session ended");
        assert!(out.events.contains(&Event::AgentExited {
            agent: id.clone(),
            pane: Some(pane.clone())
        }));
        assert!(
            m.agent(&id).is_none(),
            "a session that is over leaves no record (decision record 0030)"
        );
    }

    #[test]
    fn a_new_session_in_the_same_pane_is_a_new_record_and_the_old_one_ends() {
        let (mut m, _, _, pane) = model_with_one_tab();
        let first = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s1"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        let out = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s2"),
                T1,
            )
            .unwrap()
            .unwrap();
        assert!(out.created);
        assert_ne!(out.agent, first);
        assert!(
            m.agent(&first).is_none(),
            "one agent per pane, and the session it displaced is over"
        );
        assert!(out.events.contains(&Event::AgentExited {
            agent: first.clone(),
            pane: Some(pane.clone())
        }));
        assert_eq!(m.agent_on_pane(&pane).unwrap().id, out.agent);
    }

    #[test]
    fn process_gone_removes_the_record_and_a_report_on_a_missing_pane_is_not_found() {
        let (mut m, _, _, pane) = model_with_one_tab();
        let (id, _) = m.observe_agent(&pane, AgentKind::Codex, Some(9), T0);
        let events = m.agent_process_gone(&id);
        assert_eq!(
            events,
            vec![Event::AgentExited {
                agent: id.clone(),
                pane: Some(pane.clone())
            }]
        );
        let err = m
            .report_agent(
                &PaneId("p_0000".into()),
                AgentKind::Claude,
                hook(AgentEvent::Stop, "s"),
                T1,
            )
            .unwrap_err();
        assert_eq!(err.code, crate::api::ErrorCode::NotFound);
        assert_eq!(
            err.message,
            "pane p_0000 does not exist; the hook ran outside a domux pane or the pane closed"
        );
    }

    /// A session that ends takes its record with it, and clearing a workspace takes every
    /// record in it (decision record 0030). Two routes to one removal, and neither leaves a
    /// row behind for anybody to tidy up.
    #[test]
    fn a_session_ending_and_a_workspace_clearing_both_remove_the_record() {
        let (mut m, ws, _, pane) = model_with_one_tab();
        let live = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s1"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        let out = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionEnd, "s1"),
                T0,
            )
            .unwrap()
            .unwrap();
        assert_eq!(out.to, None);
        assert!(out.events.contains(&Event::AgentExited {
            agent: live.clone(),
            pane: Some(pane.clone())
        }));
        assert!(m.agents.is_empty(), "the session is over");

        let a = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s2"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        assert_eq!(
            m.remove_agents_of_workspace(&ws),
            vec![Event::AgentExited {
                agent: a,
                pane: Some(pane)
            }]
        );
        assert!(m.agents.is_empty());
    }

    #[test]
    fn sorted_agents_follow_interface_spec_6_7_and_the_count_is_the_waiting_ones() {
        let (mut m, ws, _, pane) = model_with_one_tab();
        let mut ids = Vec::new();
        for i in 0..8 {
            let (t, p, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
            let _ = t;
            ids.push((i, p));
        }
        let _ = pane;
        let mk = |m: &mut Model, p: &PaneId, sid: &str, t: &str| {
            m.report_agent(p, AgentKind::Claude, hook(AgentEvent::SessionStart, sid), t)
                .unwrap()
                .unwrap()
                .agent
        };
        let idle_seen = mk(&mut m, &ids[0].1, "idle-seen", "2026-09-04T14:00:00+00:00");
        let working_old = mk(
            &mut m,
            &ids[1].1,
            "working-old",
            "2026-09-04T14:01:00+00:00",
        );
        m.report_agent(
            &ids[1].1,
            AgentKind::Claude,
            hook(AgentEvent::UserPromptSubmit, "working-old"),
            "2026-09-04T14:01:00+00:00",
        )
        .unwrap()
        .unwrap();
        let working_new = mk(
            &mut m,
            &ids[2].1,
            "working-new",
            "2026-09-04T14:02:00+00:00",
        );
        m.report_agent(
            &ids[2].1,
            AgentKind::Claude,
            hook(AgentEvent::UserPromptSubmit, "working-new"),
            "2026-09-04T14:05:00+00:00",
        )
        .unwrap()
        .unwrap();
        let waiting = mk(&mut m, &ids[3].1, "waiting", "2026-09-04T14:03:00+00:00");
        m.report_agent(
            &ids[3].1,
            AgentKind::Claude,
            hook(AgentEvent::Notification, "waiting"),
            "2026-09-04T14:03:00+00:00",
        )
        .unwrap()
        .unwrap();
        let idle_unseen = mk(
            &mut m,
            &ids[4].1,
            "idle-unseen",
            "2026-09-04T14:04:00+00:00",
        );
        m.report_agent(
            &ids[4].1,
            AgentKind::Claude,
            hook(AgentEvent::UserPromptSubmit, "idle-unseen"),
            "2026-09-04T14:04:00+00:00",
        )
        .unwrap()
        .unwrap();
        m.report_agent(
            &ids[4].1,
            AgentKind::Claude,
            hook(AgentEvent::Stop, "idle-unseen"),
            "2026-09-04T14:04:30+00:00",
        )
        .unwrap()
        .unwrap();
        // Compacting shares the working rank, so its place is decided by last activity
        // alone: between the two working records rather than beside them.
        let compacting = mk(&mut m, &ids[6].1, "compacting", "2026-09-04T14:03:30+00:00");
        m.report_agent(
            &ids[6].1,
            AgentKind::Claude,
            hook(AgentEvent::PreCompact, "compacting"),
            "2026-09-04T14:03:30+00:00",
        )
        .unwrap()
        .unwrap();
        let (unknown, _) = m.observe_agent(
            &ids[5].1,
            AgentKind::Codex,
            None,
            "2026-09-04T14:07:00+00:00",
        );
        let order: Vec<AgentId> = m.sorted_agents().iter().map(|a| a.id.clone()).collect();
        assert_eq!(
            order,
            vec![
                waiting,
                working_new,
                compacting,
                working_old,
                idle_unseen,
                idle_seen,
                unknown
            ],
            "waiting, working and compacting by last activity, then unseen idle, idle and \
             unknown, each group newest first"
        );
        assert_eq!(
            m.red_dot_count(),
            1,
            "the waiting one alone: unseen no longer counts, so an idle record you have not \
             looked at carries no red dot"
        );
    }

    #[test]
    fn resolve_agent_target_accepts_ids_workspaces_and_tab_qualifiers_and_lists_candidates() {
        let (mut m, ws, tab, pane) = model_with_one_tab();
        m.rename_tab(&tab, Some("pr1".into())).unwrap();
        let a = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s1"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        assert_eq!(m.resolve_agent_target(a.as_str()).unwrap(), a);
        assert_eq!(
            m.resolve_agent_target("main").unwrap(),
            a,
            "one live agent in the workspace"
        );
        let (t2, p2, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.rename_tab(&t2, Some("tests".into())).unwrap();
        let b = m
            .report_agent(
                &p2,
                AgentKind::Codex,
                hook(AgentEvent::SessionStart, "s2"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        let err = m.resolve_agent_target("main").unwrap_err();
        assert_eq!(err.code, crate::api::ErrorCode::Ambiguous);
        assert_eq!(
            err.message,
            "2 agents are in main; qualify with the tab, for example \"main/pr1\", or use an agent id"
        );
        assert_eq!(err.data.unwrap().as_array().unwrap().len(), 2);
        assert_eq!(m.resolve_agent_target("main/pr1").unwrap(), a);
        assert_eq!(m.resolve_agent_target("main/tests").unwrap(), b);
        let err = m.resolve_agent_target("a_ffff").unwrap_err();
        assert_eq!(err.message, "agent a_ffff does not exist; run domux2 peek");
        m.report_agent(
            &p2,
            AgentKind::Codex,
            hook(AgentEvent::SessionEnd, "s2"),
            T0,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            m.resolve_agent_target("main").unwrap(),
            a,
            "exited agents are not live targets"
        );
    }

    /// Plan assumption 8: a pane hosts one live agent, so a hook from a second kind starts a
    /// new record and the first stays listed as exited with everything it had.
    #[test]
    fn a_hook_of_another_kind_on_the_pane_starts_a_new_record_and_ends_the_old_one() {
        let (mut m, _, _, pane) = model_with_one_tab();
        let claude = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s1"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        m.set_agent_recap(&claude, Some("Wrote the migration".into()));
        let out = m
            .report_agent(
                &pane,
                AgentKind::Codex,
                hook(AgentEvent::SessionStart, "s2"),
                T1,
            )
            .unwrap()
            .unwrap();
        assert!(out.created);
        assert_ne!(out.agent, claude);
        assert!(out.events.contains(&Event::AgentExited {
            agent: claude.clone(),
            pane: Some(pane.clone())
        }));
        assert!(
            m.agent(&claude).is_none(),
            "the session it displaced is over, so its record is gone"
        );
        assert_eq!(m.agent_on_pane(&pane).unwrap().kind, AgentKind::Codex);
        assert_eq!(m.agents.len(), 1, "one agent per pane");
    }

    /// A session that moves pane finds a placeholder the observer left there. The placeholder
    /// is dropped rather than ended: it never ran a session of its own, so there is nothing to
    /// remember and no pane worth naming on its way out.
    #[test]
    fn a_session_arriving_on_a_pane_drops_the_placeholder_it_finds_there() {
        let (mut m, _, _, first) = model_with_one_tab();
        let (second, _) = m
            .split_pane(&first, Direction::Right, PathBuf::from("/x"))
            .unwrap();
        let claude = m
            .report_agent(
                &first,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s1"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        let (placeholder, _) = m.observe_agent(&second, AgentKind::Codex, Some(7), T1);
        let out = m
            .report_agent(
                &second,
                AgentKind::Claude,
                hook(AgentEvent::UserPromptSubmit, "s1"),
                T1,
            )
            .unwrap()
            .unwrap();
        assert_eq!(out.agent, claude, "the session is the same record");
        assert!(
            m.agent(&placeholder).is_none(),
            "the guess is gone, not kept"
        );
        assert!(out.events.contains(&Event::AgentExited {
            agent: placeholder.clone(),
            pane: None
        }));
        assert_eq!(m.agent(&claude).unwrap().pane, Some(second));
        assert_eq!(m.agents.len(), 1);
    }

    /// Interface spec 6.5: focus and input clear the dot, and nothing else does. The recap is
    /// the one that matters, because it arrives on the same hook that turned the dot on.
    #[test]
    fn a_recap_a_name_a_pid_and_another_pane_all_leave_unseen_alone() {
        let (mut m, ws, _, pane) = model_with_one_tab();
        let (_, other, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let id = m
            .report_agent(
                &pane,
                AgentKind::Claude,
                hook(AgentEvent::SessionStart, "s1"),
                T0,
            )
            .unwrap()
            .unwrap()
            .agent;
        m.report_agent(
            &pane,
            AgentKind::Claude,
            hook(AgentEvent::Notification, "s1"),
            T1,
        )
        .unwrap()
        .unwrap();
        assert!(m.agent(&id).unwrap().unseen, "waiting turned the dot on");
        m.set_agent_recap(&id, Some("Read the transcript".into()));
        assert!(
            m.agent(&id).unwrap().unseen,
            "the recap arrives with the turn, not with you reading it"
        );
        m.set_agent_name(&id, Some("auth cleanup".into()));
        assert!(
            m.agent(&id).unwrap().unseen,
            "naming a session is not seeing it"
        );
        m.set_agent_pid(&id, Some(99));
        assert!(m.agent(&id).unwrap().unseen);
        let elsewhere = m
            .report_agent(
                &other,
                AgentKind::Codex,
                hook(AgentEvent::SessionStart, "s2"),
                T1,
            )
            .unwrap()
            .unwrap()
            .agent;
        assert!(
            m.clear_unseen_for_pane(&other).is_empty(),
            "the other pane has no dot to clear"
        );
        m.clear_unseen(&elsewhere);
        assert!(
            m.agent(&id).unwrap().unseen,
            "another pane's agent does not clear this one"
        );
        assert_eq!(
            m.clear_unseen_for_pane(&pane),
            vec![Event::AgentUnseenChanged {
                agent: id.clone(),
                unseen: false
            }],
            "focusing the pane is what clears it"
        );
        assert!(!m.agent(&id).unwrap().unseen);
    }

    /// An agent id outlives its record: a client's `agents_cursor` holds one, and so do the
    /// server's per-agent working word and recap. An id handed back would show a dead
    /// session's recap under a live agent, which is the aliasing `retired` exists to stop.
    ///
    /// Each removal path is checked by rewinding the generator to the stream that already
    /// produced the id, so the collision is certain and each path answers for itself. Drawing
    /// fresh ids in a loop and asserting they differ would instead depend on the draw count
    /// passing the generator's first repeat, and on which branch happened to remove the
    /// earlier of the two colliding ids: a precondition that holds or fails silently.
    #[test]
    fn ids_are_not_reissued_after_a_session_ends_or_its_workspace_is_cleared() {
        for clear_the_workspace in [false, true] {
            let (mut m, ws, _, pane) = model_with_one_tab();
            let id = m
                .report_agent(
                    &pane,
                    AgentKind::Claude,
                    hook(AgentEvent::SessionStart, "s1"),
                    T0,
                )
                .unwrap()
                .unwrap()
                .agent;
            if clear_the_workspace {
                m.remove_agents_of_workspace(&ws);
            } else {
                m.report_agent(
                    &pane,
                    AgentKind::Claude,
                    hook(AgentEvent::SessionEnd, "s1"),
                    T0,
                )
                .unwrap()
                .unwrap();
            }
            assert!(m.agents.is_empty());
            // The fixture seeds the generator with 7 and this id was its fifth draw, so
            // rewinding offers that exact value again within the next few.
            m.reseed(7);
            let redrawn: Vec<String> = (0..8).map(|_| m.next_id("a").unwrap()).collect();
            assert!(
                !redrawn.contains(&id.0),
                "a removed agent's id {id} came back as {redrawn:?} \
                 (clear_the_workspace: {clear_the_workspace})"
            );
        }
    }

    /// The same route, checked for the id: a dropped placeholder never gives its id back.
    #[test]
    fn ids_are_not_reissued_after_an_arriving_session_drops_a_placeholder() {
        let (mut m, _, _, first) = model_with_one_tab();
        let (second, _) = m
            .split_pane(&first, Direction::Right, PathBuf::from("/x"))
            .unwrap();
        m.report_agent(
            &first,
            AgentKind::Claude,
            hook(AgentEvent::SessionStart, "s1"),
            T0,
        )
        .unwrap()
        .unwrap();
        let (placeholder, _) = m.observe_agent(&second, AgentKind::Codex, Some(7), T1);
        m.report_agent(
            &second,
            AgentKind::Claude,
            hook(AgentEvent::UserPromptSubmit, "s1"),
            T1,
        )
        .unwrap()
        .unwrap();
        assert!(m.agent(&placeholder).is_none());
        m.reseed(7);
        let redrawn: Vec<String> = (0..8).map(|_| m.next_id("a").unwrap()).collect();
        assert!(
            !redrawn.contains(&placeholder.0),
            "a dropped placeholder's id {placeholder} came back as {redrawn:?}"
        );
    }

    /// A target that already names a tab has no second qualifier to offer, so the message
    /// asks for an id rather than printing `main/pr1/pr1`, which resolves to nothing.
    #[test]
    fn an_already_qualified_target_asks_for_an_id_rather_than_a_second_tab() {
        let (mut m, _, tab, pane) = model_with_one_tab();
        m.rename_tab(&tab, Some("pr1".into())).unwrap();
        let (split, _) = m
            .split_pane(&pane, Direction::Right, PathBuf::from("/x"))
            .unwrap();
        m.report_agent(
            &pane,
            AgentKind::Claude,
            hook(AgentEvent::SessionStart, "s1"),
            T0,
        )
        .unwrap()
        .unwrap();
        m.report_agent(
            &split,
            AgentKind::Codex,
            hook(AgentEvent::SessionStart, "s2"),
            T0,
        )
        .unwrap()
        .unwrap();
        let err = m.resolve_agent_target("main/pr1").unwrap_err();
        assert_eq!(err.code, crate::api::ErrorCode::Ambiguous);
        assert_eq!(err.message, "2 agents are in main/pr1; use an agent id");
        assert_eq!(err.data.unwrap().as_array().unwrap().len(), 2);
    }
}
