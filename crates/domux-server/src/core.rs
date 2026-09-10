//! The core task: the one owner of the Model, every PaneRuntime and every ClientConn.

use crate::agents::observer::{self, PaneProcess};
use crate::api::{self, Ctx};
use crate::client::{ClientConn, Hint, HintKind};
use crate::facts::FactRegistry;
use crate::pane::{new_pane_emulator, PaneRuntime, SpawnRequest, PANE_TERM};
use crate::process::ForegroundProcess;
use crate::render::{self, RenderInput};
use crate::worktree_conf;
use crate::{CoreDeps, LoadedConfig, ServerOptions};
use chrono::{DateTime, Local};
use domux_core::api::{ApiError, ErrorCode, Event, Method, Request, Response};
use domux_core::config::ResumeMode;
use domux_core::facts::{Fact, FactKey, FactState};
use domux_core::ids::{AgentId, ClientId, PaneId, ProjectId, TabId, WorkspaceId};
use domux_core::keymap::Action;
use domux_core::model::agent::AgentState;
use domux_core::model::{
    ClientView, ConfirmKind, Focus, Model, Overlay, PaneFacts, Pill, RegionKind, PILL_SECONDS,
};
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
    /// One provider's answer, from the blocking task that ran it. `None` is an absence:
    /// the provider failed, or it looked and there was nothing to report.
    FactFetched {
        key: FactKey,
        fact: Option<Fact>,
    },
    /// Once a second: the process inspector, the clock, exited-pane cleanup.
    Tick,
    /// One frame of the working glyph, every `agents::labels::GLYPH_INTERVAL`. The ticker
    /// owns nothing and never stops; the core counts, and ignores the message while nothing
    /// is working (M3 plan assumption 35).
    AnimationTick,
    /// A job that shelled out has finished. The model changes here, on the core task, and
    /// the caller waiting on `reply` is answered (decision record 0006).
    JobFinished {
        outcome: JobOutcome,
        reply: Option<JobReply>,
        client: Option<ClientId>,
        /// What the job held while it ran, to be released now that it is over. `None` for a
        /// job that only reads. See `Core::claims`.
        claim: Option<String>,
    },
    Snapshot {
        reply: oneshot::Sender<Model>,
    },
    Shutdown,
}

/// Work that shells out. It runs on a blocking task; the API reply, when there is one,
/// travels with it, so the caller waits and the core does not (decision record 0006).
pub enum CoreJob {
    /// Everything `project.add` needs from git and the filesystem, read off the core task.
    ReadProject { path: String },
    /// The worktree, the branch and the `worktree.conf` setup of one new slot. `base` is
    /// what the caller or the configuration asked for, not the ref it resolves to: reading
    /// `origin/HEAD` is a fork, so `git::base_ref` runs here rather than in the handler.
    CreateWorkspace {
        project: ProjectId,
        root: PathBuf,
        slot: u32,
        path: PathBuf,
        branch: String,
        base: Option<String>,
    },
    /// Puts a slot back at its base: `git reset --hard` and `git clean -fd` in its worktree.
    /// The model is not touched, so nothing here comes back but the base it reset to.
    ClearWorkspace {
        workspace: WorkspaceId,
        /// What the refusal and the pill call the slot. The job runs off the core task and
        /// cannot read the model, so the display name travels with it.
        name: String,
        root: PathBuf,
        path: PathBuf,
        /// `[worktrees] base`, not the ref it resolves to: `git::base_ref` reads
        /// `origin/HEAD`, which is a fork.
        base: Option<String>,
        /// False means "refuse if there is anything in the slot to lose". Only
        /// `git::is_dirty` can answer that, and it shells out, so the gate is here.
        yes: bool,
    },
    /// Removes a slot's worktree and its local branch. The record goes in the `Deleted` arm,
    /// on the core task, after the work on disk has all worked.
    DeleteWorkspace {
        workspace: WorkspaceId,
        name: String,
        root: PathBuf,
        path: PathBuf,
        /// The branch the confirmation named, when it named one. The job reads the branch
        /// itself and refuses if the two disagree: the reader consented to losing a named
        /// branch, and losing a different one because the slot moved in between is the
        /// surprise the question exists to prevent (principle 10).
        ///
        /// `None` when no branch fact had arrived, which is when the question said "its local
        /// branch" and promised nothing specific. There is then nothing to reconcile and the
        /// fresh read stands.
        ///
        /// **This holds on the key path and not on the shell path**, and the difference is not
        /// something this field can close. A key asks and is answered in one session, so the
        /// name the box drew is the name that arrives here. A shell prints the question in one
        /// process and takes `--yes` in another, so the second call reads the fact again, gets
        /// whatever it says by then, and has nothing to compare against. The fact's time to
        /// live is 30 seconds and it refreshes on its own, so that window closes with no
        /// action from the author.
        ///
        /// What that costs is bounded and worth stating exactly: the job always removes the
        /// branch the worktree is really on, which is the right branch for that worktree, so a
        /// shell reader can be told one name and lose a different, correct one. That is
        /// misinformation, not misdeletion. `JobOutcome::Deleted::branch` carries what really
        /// went so the answer names it, and carrying the name through the shell path would
        /// mean the refusal printing a command that includes the branch, which adds a
        /// parameter the command table fixes. Recorded as a follow-up rather than done here.
        expected_branch: Option<String>,
        base: Option<String>,
        /// False means "refuse a dirty slot". `git worktree remove` has a force of its own
        /// for modified and untracked files, but it does not see commits that were never
        /// pushed, so `git::is_dirty` in front of it is the only guard against those.
        force: bool,
    },
}

impl CoreJob {
    /// What this job holds for as long as it runs, so a second call cannot choose the same
    /// thing before this one has recorded it. `None` for a job that only reads the world and
    /// lets its arm decide, which the core task already serialises (decision record 0006).
    fn claim(&self) -> Option<String> {
        match self {
            CoreJob::ReadProject { .. } => None,
            CoreJob::CreateWorkspace { project, slot, .. } => Some(slot_claim(project, *slot)),
            // Neither chooses anything. Both are told which workspace to act on and both read
            // the world as they find it, and the core task serialises the arms that write the
            // model, so a second call cannot act on a stale answer (decision record 0006).
            // Two deletes of one slot do overlap in git, and the second one loses: its
            // `git worktree remove` fails and its arm never runs, which is loud and changes
            // nothing. A claim would make that a `busy` refusal instead; it is not built,
            // because nothing is corrupted by the answer that arrives today.
            CoreJob::ClearWorkspace { .. } | CoreJob::DeleteWorkspace { .. } => None,
        }
    }
}

/// The claim one slot number of one project stands for. Written here and read in
/// `api::workspace::create`, so the two cannot spell it differently.
pub fn slot_claim(project: &ProjectId, slot: u32) -> String {
    format!("{project} workspace-{slot}")
}

/// What `worktree.conf` did for a new slot: the links and copies that were applied, and the
/// run lines for its first pane. `None` in `Created` means the project has no
/// `worktree.conf` at all, which is not the same as one that asked for nothing (principle 4).
pub struct Setup {
    pub applied: worktree_conf::Applied,
    pub run: Vec<String>,
}

/// What a job found. Nothing here has touched the model: the `JobFinished` arm does that.
pub enum JobOutcome {
    /// What `ReadProject` found. `default_branch` is `None` for a plain folder, and `slots`
    /// is empty for one. Each slot carries the directory that is really on disk, which is
    /// not always `git::slot_path`: V1 wrote some of them under `.baag/worktrees`.
    ProjectRead {
        root: PathBuf,
        default_branch: Option<String>,
        slots: Vec<(u32, PathBuf)>,
    },
    /// The worktree `CreateWorkspace` made, with everything the record and the answer need.
    /// `base` is the ref it really branched from, which is why it comes back rather than
    /// being worked out again here.
    Created {
        project: ProjectId,
        slot: u32,
        path: PathBuf,
        branch: String,
        base: String,
        setup: Option<Setup>,
    },
    /// The slot `ClearWorkspace` reset, and the ref it was reset to. The base comes back
    /// rather than being worked out again here: `git::base_ref` resolved it on the job.
    Cleared {
        workspace: WorkspaceId,
        name: String,
        base: String,
    },
    /// The slot `DeleteWorkspace` removed from disk. Its record is still in the model when
    /// this arrives; the arm is what takes it.
    Deleted {
        workspace: WorkspaceId,
        name: String,
        /// What was really removed, not what the question proposed. The two can differ for a
        /// caller with a command line (`CoreJob::DeleteWorkspace::expected_branch`), and a
        /// reader who is only ever told the proposal has no way to find that out.
        branch: String,
    },
    Failed {
        message: String,
        code: ErrorCode,
    },
}

/// What one dispatched method produced besides its answer.
struct Dispatched {
    result: Result<serde_json::Value, ApiError>,
    /// What the handler queued to be run off the core task.
    jobs: Vec<CoreJob>,
    /// The handler queued a job that carries the answer, so `result` is not the answer.
    deferred: bool,
}

/// The waiting caller's answer, carried with the job that will produce it.
///
/// The request id travels too. A `Response` cannot be built without one, and by the time
/// the job finishes the `Request` it came from is gone, so the id has to be captured when
/// the reply is deferred rather than looked up later.
pub struct JobReply {
    pub id: serde_json::Value,
    pub tx: oneshot::Sender<Response>,
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

/// What a frame shows of a fact, which is what decides whether an answer is news. The stamp
/// and the time to live are not part of it: a provider that looks again every five seconds
/// and finds the same branch must not redraw the screen five times a minute, and a pull
/// request that changed state without changing its number must.
fn shown(fact: Option<&Fact>) -> Option<(&str, Option<&FactState>, Option<&str>)> {
    fact.map(|f| (f.text.as_str(), f.state.as_ref(), f.url.as_deref()))
}

/// Whether the filesystem says there is nothing at `path`, as against saying it cannot tell.
///
/// `Path::is_dir` is `metadata().map(..).unwrap_or(false)`, so it answers false for a
/// permission denied on a parent, an `EIO` from a failing disk, a timed-out network mount and
/// a symlink loop, none of which mean the author's work is gone. `prune_missing_paths` deletes
/// records on this answer, so it asks the narrower question.
///
/// **This is not sufficient on its own, and nothing here should be read as saying it is.** An
/// external drive that is not mounted gives `NotFound` for every path under its mount point,
/// and at this layer that is indistinguishable from a directory somebody deleted. Telling the
/// two apart needs a state on the model for "missing, kept", which is a later milestone's.
/// Until then what covers the case is the copy `keep_pre_prune_state` leaves behind, so do not
/// remove that on the strength of this check.
///
/// A path that exists but is not a directory is not missing either. Something is there, and
/// deleting the record of it is the outcome this whole function exists to avoid.
fn is_missing(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            tracing::warn!(
                "keeping the record for {}: the filesystem could not answer for it ({e})",
                path.display()
            );
            false
        }
        Ok(_) => false,
    }
}

/// Copies the state file to `<path>.pruned` before the first record is taken away.
///
/// `write_atomic` renames the current file to `.bak` on every write, so `.bak` holds the
/// pre-prune model for exactly one further write - and `publish_events` treats the prune's own
/// events as structural, so the first write happens within milliseconds and the attaching
/// client's `PaneResized` takes the second. `.pruned` is a name nothing rotates, the same
/// answer `Core::new` already gives a state file it refused to parse (`.rejected`).
///
/// A copy and not a rename: the file the server is about to keep writing has to stay where it
/// is. Each start overwrites the previous `.pruned`, which is the right trade for a start that
/// only writes one when it is about to delete something.
///
/// A failure here does not stop the prune. The model has to match the disk either way, and a
/// start that refused to correct itself because it could not write a backup would be a server
/// that will not start.
fn keep_pre_prune_state(state_file: &Path) {
    let kept = crate::persist::with_suffix(state_file, ".pruned");
    match std::fs::copy(state_file, &kept) {
        Ok(_) => tracing::warn!(
            "records are about to be pruned; the state as it was is kept at {}",
            kept.display()
        ),
        // Not an error at `warn`: the ordinary way here is a first start, which has no state
        // file to copy and nothing to prune either.
        Err(e) => tracing::info!(
            "the state file at {} was not copied to {} ({e})",
            state_file.display(),
            kept.display()
        ),
    }
}

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
    /// Published beside `snapshot`: see `ServerHandle::facts`.
    published_facts: Arc<Mutex<HashMap<FactKey, Fact>>>,
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
    /// What domux observed. The core reads it and records answers; the fetching happens on
    /// blocking tasks.
    pub facts: FactRegistry,
    /// What the agent handlers hold outside the Model: working words, the transcript reader
    /// and the manifest registry.
    pub agents: crate::agents::AgentsState,
    /// What the jobs in flight have chosen and not yet written into the model.
    ///
    /// A handler that only reads the model is safe without this, because the core task
    /// serialises the arms that write it. A handler that **chooses** is not: it chooses when
    /// the call arrives and its job records the choice seconds later, so a second call in
    /// between reads a model that says the thing is still free (decision record 0006).
    /// `workspace.create` is that shape, and this is what makes two of them at once pick two
    /// different slot numbers instead of both picking the lowest.
    ///
    /// Keys are strings so the mechanism is not slot-shaped: `CoreJob::claim` names what a
    /// job holds. `start_job` is the only writer and `job_finished` the only remover, so a
    /// claim cannot be taken without a job to release it: a job that panics still sends
    /// `JobFinished`, and every path out of an arm runs after the claim is already gone.
    ///
    /// A job that never finishes at all is the one case this does not cover. `spawn_blocking`
    /// has no deadline, so a `git fetch` against a host that swallows packets holds its slot
    /// number until the server stops, and later creates skip past it. The caller hangs on the
    /// same job, so it is visible rather than silent, and the deadline belongs to the lane
    /// (decision record 0006) rather than to the claim.
    claims: HashSet<String>,
    /// What the start-up prune took away, in the words the switcher's footer and the
    /// sidebar's hint row print. One list for the server rather than one per client: the
    /// prune runs before any client attaches, so a per-client list would be empty for
    /// everybody.
    ///
    /// Cleared by the first key in a box, so a note is read once - and because the list is the
    /// server's, **whichever** client presses that key clears it for every attached client.
    /// That is the difference from a pill, which is one client's and ages out instead. Two
    /// readers at two screens therefore share one note, and the first of them to look at a
    /// list takes it away from the other.
    notes: Vec<String>,
}

impl Core {
    pub fn new(
        opts: ServerOptions,
        core_tx: mpsc::Sender<CoreMsg>,
        persist_tx: mpsc::Sender<StateFile>,
        state_file: &Path,
        snapshot: Arc<Mutex<Model>>,
        pane_sizes: Arc<Mutex<HashMap<PaneId, Size>>>,
        published_facts: Arc<Mutex<HashMap<FactKey, Fact>>>,
    ) -> anyhow::Result<Core> {
        let started_at = opts.deps.clock.now().to_rfc3339();
        let project_root = opts.project_root.clone();
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
        let mut facts = FactRegistry::new();
        for provider in opts.providers {
            facts.register(provider);
        }
        // The pull request survives a restart, so the switcher opens with the last known
        // number instead of a blank line. Anything past its time to live, and anything about
        // a workspace or a project this model no longer holds, is dropped rather than shown.
        facts.load_cache(
            &crate::facts::pr_cache_path(&opts.state_dir),
            opts.deps.clock.now(),
        );
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
            published_facts,
            view_dirty: true,
            last_minute: None,
            stopping: false,
            pane_started_at: HashMap::new(),
            immediate_exits: HashMap::new(),
            respawn_blocked: HashSet::new(),
            facts,
            claims: HashSet::new(),
            notes: Vec::new(),
            agents: crate::agents::AgentsState::default(),
        };
        // Before the seed below and before anything is spawned or resumed. A record whose
        // path is gone must not reach `ensure_every_workspace_has_a_tab`, which would give it
        // a tab, or `spawn_pane`, which would start a shell in a directory that is not there.
        core.notes = core.prune_missing_paths(state_file);
        // The directory the server was started in, when the model holds nothing else. After
        // the prune rather than before it: a state file whose every project has been removed
        // with `rm -rf` prunes down to nothing, and a server with no workspace refuses every
        // client at `attach`, which leaves it running and unusable with no way back but
        // deleting the state file.
        if core.model.projects.is_empty() {
            // A model with no projects holds at most `RETIRED_CAPACITY` ids of a possible
            // 65536, so this cannot exhaust the id space.
            core.model
                .add_folder_project(project_root)
                .expect("a model with no projects cannot exhaust the id space");
        }
        // Last of the three, so it judges the model the server is actually starting with:
        // anything the cache knew about a record the prune took away, and nothing about the
        // project that was just seeded. Run before the prune it would keep a pruned
        // workspace's pull request, which the cache file would then carry for ever; run
        // before the seed it would drop the seeded project's own facts on the way past an
        // empty model.
        core.facts.forget_deleted(&core.model);
        core.ensure_every_workspace_has_a_tab();
        for pane in core.model.all_pane_ids() {
            core.spawn_pane(&pane, Size { cols: 80, rows: 24 });
        }
        core.resume_agents_on_start();
        core.pending_events.push(Event::ServerStarted {
            version: domux_core::VERSION.into(),
            socket: core.socket_path.clone(),
        });
        Ok(core)
    }

    /// `[resume] agents = "auto"`: every record the server starts holding gets its relaunch line
    /// typed into its pane (architecture spec section 5). `manual` is the default and this does
    /// nothing.
    ///
    /// At start only, and not on every attach (plan assumption 30). A reader who attaches a
    /// second terminal to a running server has not asked for anything to be relaunched, and
    /// resuming on attach would type a second `claude --resume` into a pane where the first one
    /// is already running.
    ///
    /// One `workspace.resume` per workspace, which is every record the model holds: a record's
    /// workspace is one the model has, because `state_file::restore` drops a record whose
    /// workspace is gone rather than restoring it dangling.
    ///
    /// **`workspace.resume` and not one `agent.resume` per record**, because resuming a set of
    /// records is not the same operation as resuming one, and the difference is a rule that has to
    /// live in one place: a pane takes one relaunch line, so a set has to keep the first record for
    /// each pane and skip the rest. That rule needs to know what the loop has already typed, which
    /// `agent.resume` cannot know and `plan_resume` must not, so it belongs to a loop - and one
    /// loop is better than two copies of it.
    ///
    /// It carries no filter of what may be resumed either, and for the same reason: one rule, one
    /// home. `agent::plan_resume` is the one judge, `workspace.resume` collects its refusals, and a
    /// live record reaching this would produce a skipped line rather than a write. That does not
    /// lean on `state_file::restore` exiting every live record it reads, which is that function's
    /// behaviour today rather than a promise to this one.
    ///
    /// This runs after the pane loop above because the line goes into a shell, and until each
    /// pane has been spawned there is no shell to type into.
    ///
    /// Everything it could not do is logged, because there is nobody to tell: no client has
    /// attached - `Core::new` returns before `socket::listen` runs - so there is no screen for a
    /// pill and no caller waiting for a reply.
    fn resume_agents_on_start(&mut self) {
        if self.config.config.resume.agents != ResumeMode::Auto {
            return;
        }
        let workspaces: Vec<WorkspaceId> = self
            .model
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .map(|w| w.id.clone())
            .collect();
        for workspace in workspaces {
            let method = Method::WorkspaceResume(domux_core::api::WorkspaceTargetParams {
                workspace: Some(workspace.to_string()),
            });
            match self.dispatch(method, None) {
                Ok(value) => {
                    for line in value
                        .get("skipped")
                        .and_then(|s| s.as_array())
                        .map(|s| s.as_slice())
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|line| line.as_str())
                    {
                        tracing::info!(workspace = %workspace, "not resumed at start: {line}");
                    }
                }
                Err(e) => {
                    tracing::info!(workspace = %workspace, "not resumed at start: {}", e.message)
                }
            }
        }
    }

    /// Records whose path is gone are removed at start, before any pane is spawned, and the
    /// reason is kept for the switcher's footer and the sidebar's hint row (architecture spec
    /// section 5). The author who removed a worktree or a project folder with `rm -rf` gets a
    /// model that matches the disk again, and one line saying so.
    ///
    /// The project loop runs first on purpose. A `main` workspace's path is the project's
    /// root, so when the root is gone the project is gone and takes its `main` with it.
    /// `prune_workspace` refuses `main` on its own, which is why the workspace loop reads
    /// `if let Ok`: by the time it runs, every surviving `main` has a folder, so the refusal
    /// is unreachable rather than swallowed.
    ///
    /// That last argument holds **while** a `main` workspace's path is its project's root,
    /// which only construction guarantees. `add_project` sets the two equal and nothing writes
    /// either afterwards, so it is true of every state file this build wrote; but
    /// `state_file::restore` deserializes a `Project` straight from JSON and checks only that
    /// each tab's focused pane is in its layout, so a hand-edited file where they differ
    /// reaches the swallowed `if let Ok` and keeps a record it should have taken. If that ever
    /// becomes reachable, skip `WorkspaceHandle::Main` in the workspace filter and the refusal
    /// is unreachable by construction here too.
    ///
    /// Nothing here retires an id. `remove_project` and `prune_workspace` retire every
    /// workspace, tab and pane id they take away, so a prune at start cannot hand an id back
    /// out to a different object later in the session. One place owns that and it is the
    /// model.
    ///
    /// What is lost when this is wrong is not files: `remove_project` leaves the folder and
    /// its worktrees on disk (interface spec 12.8). It is every **record** - workspace names,
    /// tab and pane layouts, saved directories, `last_workspace` - and nothing here asks
    /// first, because at start there is nobody to ask. `project.remove`, which is the same
    /// deletion asked for deliberately, is guarded by a confirmation and by `--yes`. So this
    /// path carries three defences instead: it prunes only on `NotFound`, it logs each record
    /// with its path, and it keeps the pre-prune state file where nothing rotates it.
    fn prune_missing_paths(&mut self, state_file: &Path) -> Vec<String> {
        let mut notes = Vec::new();
        // Copied at most once, and only when something is actually about to go, so a clean
        // start does not rotate a file for nothing. Copying after the project loop still gets
        // the pre-prune file: nothing writes `state.json` during `Core::new`, because the only
        // writer is the persistence task and `publish_events` does not run until the batch
        // loop, after this returns.
        let mut kept = false;
        let gone_projects: Vec<_> = self
            .model
            .projects
            .iter()
            .filter(|p| is_missing(&p.root))
            .map(|p| (p.id.clone(), p.name.clone(), p.root.clone()))
            .collect();
        if !gone_projects.is_empty() {
            keep_pre_prune_state(state_file);
            kept = true;
        }
        for (id, name, root) in gone_projects {
            if let Ok(events) = self.model.remove_project(&id) {
                self.pending_events.extend(events);
                tracing::warn!(
                    "removing project {name}: {} is not there. The folder and any worktrees under it are left alone",
                    root.display()
                );
                notes.push(format!("Removed {name}: its folder is gone"));
            }
        }
        let gone: Vec<_> = self
            .model
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .filter(|w| is_missing(&w.path))
            .map(|w| (w.id.clone(), w.display_name(), w.path.clone()))
            .collect();
        if !gone.is_empty() && !kept {
            keep_pre_prune_state(state_file);
        }
        for (id, name, path) in gone {
            if let Ok((_, events)) = self.model.prune_workspace(&id) {
                self.pending_events.extend(events);
                tracing::warn!("pruning workspace {name}: {} is not there", path.display());
                notes.push(format!("Pruned {name}: its worktree is gone"));
            }
        }
        notes
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
                let observed = self.panes.get(pane).map(|rt| self.observe_pane(rt).0);
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
                    // Here rather than on the next tick: the child is gone, so every agent
                    // that was running in this pane is gone with it, and a row must not read
                    // `working` for up to a second after the process it names ended.
                    self.agents_gone_with_pane(&pane);
                }
            }
            CoreMsg::ClientConnected { hello, tx, reply } => {
                let _ = reply.send(self.attach(hello, tx));
            }
            CoreMsg::ClientInput { client, msg } => self.client_input(client, msg),
            CoreMsg::ClientGone { client } => self.detach(&client, None),
            CoreMsg::Api { request, reply } => self.api(request, reply),
            CoreMsg::FactFetched { key, fact } => {
                // A workspace removed while its provider was still running: the answer is
                // about nothing the model holds, so it is dropped rather than put back.
                let fact = fact.filter(|_| crate::facts::scope_lives(&key, &self.model));
                let present = fact.is_some();
                let changed = shown(self.facts.get(&key)) != shown(fact.as_ref());
                let cached = crate::facts::CACHED_FACTS.contains(&key.name.as_str());
                self.facts.set(key.clone(), fact);
                if changed {
                    self.pending_events
                        .push(Event::FactUpdated { key, present });
                    self.view_dirty = true;
                }
                // Every answer, not only one that changed what the screen shows. `changed`
                // compares what is drawn, which deliberately ignores the stamp, so a pull
                // request found every minute that never changes would keep a fresh stamp in
                // the registry and the first fetch's stamp on disk. After a run longer than
                // the time to live the next start would drop the entry and the switcher would
                // open blank, which is the one thing the cache exists to prevent. One small
                // file a minute per workspace is the price, and it is an atomic write.
                if cached {
                    self.facts.save_cache(
                        &crate::facts::pr_cache_path(&self.state_dir),
                        crate::facts::CACHED_FACTS,
                    );
                }
            }
            CoreMsg::Subscribe { filter, tx } => self.subscribers.push((filter, tx)),
            CoreMsg::Tick => self.tick(),
            CoreMsg::AnimationTick => self.animation_tick(),
            CoreMsg::JobFinished {
                outcome,
                reply,
                client,
                claim,
            } => self.job_finished(outcome, reply, client, claim),
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
            // The remembered state, so a new client opens the screen the last one left
            // (roadmap decision 4).
            sidebar_open: self.model.sidebar_open,
            sidebar_forced: false,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            agents_cursor: None,
            agents_scroll: 0,
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
            ClientMsg::Scroll { column, row, lines } => {
                self.clear_action_hint(&client);
                self.scroll(&client, column, row, lines);
            }
            ClientMsg::Paste(text) => {
                if let Some(pane) = self.focused_pane(&client) {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        let mut out = Vec::new();
                        p.emulator.encode_paste(&text, &mut out);
                        p.write(&out);
                    }
                    self.seen_by_input(&pane);
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
    ///
    /// A press the pane took is the reader typing into it, which clears the dot on the agent
    /// there (interface spec 6.5). Here rather than in `input::forward_to_pane`, because copy
    /// mode and an exited pane take the key without going through it and a reader in either
    /// is reading that pane. Presses only: a release arrives after whichever key it belongs
    /// to and always routes to the pane, so clearing on one would take the dot away for
    /// `leader a` as well.
    ///
    /// The pane is read **before** the routing, because the routing can move the focus off it.
    /// Enter on a pane whose child exited closes that pane, and `Model::close_pane` hands the
    /// tab's focus to a neighbour, so reading it afterwards named a pane the key never reached
    /// and cleared the dot of whatever agent was in it. Nothing on the screen says a dot went,
    /// and only a state transition sets one (`model::agent::attention`), so the reader cannot
    /// ask for it back.
    fn key(&mut self, client: &ClientId, key: domux_term::KeyEvent) {
        self.clear_notes_read_by(client, &key);
        let press = key.action != domux_term::KeyAction::Release;
        let aimed_at = self.focused_pane(client);
        let route = crate::input::route_key(self, client, key);
        if press && route == crate::input::Route::Pane {
            if let Some(pane) = aimed_at {
                self.seen_by_input(&pane);
            }
        }
        self.view_dirty = true;
    }

    /// Input reached `pane`, so the agent there has been seen (interface spec 6.5). The one
    /// rule lives in `Model::clear_unseen_for_pane`; this is the core's way of recording what
    /// it produced, and `api::pane`'s `seen_by_input` is the handler's.
    fn seen_by_input(&mut self, pane: &PaneId) {
        let cleared = self.model.clear_unseen_for_pane(pane);
        self.view_dirty |= !cleared.is_empty();
        self.pending_events.extend(cleared);
    }

    /// Scrolls the pane whose box contains the outer terminal cell. A scroll over chrome,
    /// unused space, a size notice or an overlay belongs to none. The gesture focuses its pane
    /// only once there is history to move through.
    fn scroll(&mut self, client: &ClientId, column: u16, row: u16, lines: i16) {
        let Some(pane) = self.pane_at(client, column, row) else {
            return;
        };
        let handled = self
            .panes
            .get_mut(&pane)
            .is_some_and(|rt| crate::copy_mode::scroll(rt, lines));
        if !handled {
            return;
        }
        if let Ok(events) = self.model.focus_pane(&pane) {
            self.pending_events.extend(events);
        }
        self.model.set_pane_copy_mode(&pane, true);
        self.view_dirty = true;
    }

    fn pane_at(&self, client: &ClientId, column: u16, row: u16) -> Option<PaneId> {
        let view = self.model.client(client)?;
        if view.overlay.is_some()
            || !matches!(view.focus, Focus::Pane(_))
            || view.size.cols < render::MIN_COLS
            || view.size.rows < render::MIN_ROWS
        {
            return None;
        }
        let tab = self.model.tab(&view.tab)?;
        // The same rectangle `render::draw_panes` lays the boxes out on, so a cell hits the
        // pane the reader sees under it.
        let area = render::smallest_workpanel(&self.model, &tab.id, view);
        domux_core::model::layout::solve(&tab.layout, area, tab.zoomed.as_ref())
            .into_iter()
            .find(|(_, rect)| {
                column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
            })
            .map(|(pane, _)| pane)
    }

    /// A note is gone once the reader has been in a box with it on the screen, so it is read
    /// once and does not sit in the row for the rest of the session.
    ///
    /// In a box, and not on any key: a note also prints in the sidebar's hint row, and the
    /// sidebar stands beside a pane the reader is typing in. Clearing on any key at all would
    /// take the note away during the first keystroke of the day, which is the one moment
    /// nobody is looking at the sidebar.
    ///
    /// A release is not a key press. Both halves of a press arrive here, and clearing on the
    /// release would take the note away one event before `route_key` decided what the press
    /// meant.
    fn clear_notes_read_by(&mut self, client: &ClientId, key: &domux_term::KeyEvent) {
        if self.notes.is_empty() || key.action == domux_term::KeyAction::Release {
            return;
        }
        // One question, asked of the focus, because the rule is about a box and not about any
        // particular one. M2 wrote it out as the switcher plus the sidebar's Projects box and
        // M3 added two more places a key can land in a box, the sidebar's Agents box and the
        // agents overlay, which a list of names silently fell through. `RegionKind::is_box`
        // is the same question `render::overlay::draw_help` asks to pick a key table.
        //
        // The focus alone is enough. Every box sets its own region when it opens
        // (`api::switcher::open`, `api::agents::open`, `enter_sidebar_box`), and
        // `ClientView::focus_after_pop` names the switcher and the agents overlay again when
        // an overlay over one closes, so a box the reader came back to still reads as a box.
        let in_a_box = self
            .model
            .client(client)
            .is_some_and(|view| matches!(view.focus, Focus::Region(kind) if kind.is_box()));
        if in_a_box {
            self.notes.clear();
        }
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
                if let Err(e) = self.dispatch_from_key(method, Some(client.clone())) {
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

    /// One API request. The answer goes to `reply` here, unless the handler deferred it:
    /// a handler that has to shell out queues a job and the job carries the answer, so the
    /// caller waits and the core does not (decision record 0006).
    fn api(&mut self, request: Request, reply: oneshot::Sender<Response>) {
        let id = request.id.clone();
        let method = match Method::from_request(&request.method, request.params) {
            Ok(m) => m,
            Err(e) => {
                let _ = reply.send(Response::err(id, e));
                return;
            }
        };
        let client = param_client(&method).or_else(|| self.model.most_recent_client());
        let done = self.dispatch_inner(method, client.clone(), false);
        let mut reply = Some(reply);
        if !done.deferred {
            if let Some(tx) = reply.take() {
                let _ = tx.send(match done.result {
                    Ok(v) => Response::ok(id.clone(), v),
                    Err(e) => Response::err(id.clone(), e),
                });
            }
        }
        for (i, job) in done.jobs.into_iter().enumerate() {
            // The first job carries the deferred answer, which is all `defer_reply` can
            // mean while a handler that defers queues one job. A second job would be work
            // nobody is waiting on, and it still runs.
            let carried = match i {
                0 => reply.take().map(|tx| JobReply { id: id.clone(), tx }),
                _ => None,
            };
            self.start_job(job, carried, client.clone());
        }
        // A handler that deferred and queued nothing would leave the caller waiting for
        // ever. No handler does that today - `project.add` is the only one that defers and
        // it always queues - so nothing reaches this and no test pins it. It is here
        // because the cost of the state it guards against is a hung caller rather than a
        // wrong answer, and a hung caller is invisible in a suite.
        if let Some(tx) = reply {
            let _ = tx.send(Response::err(
                id,
                ApiError::internal("the handler deferred its answer and queued no job"),
            ));
        }
    }

    /// The one entry point for methods, from the API and from keys.
    pub fn dispatch(
        &mut self,
        method: Method,
        client: Option<ClientId>,
    ) -> Result<serde_json::Value, ApiError> {
        self.dispatch_from(method, client, false)
    }

    /// `dispatch` for a key press. The handler is told where the call came from, so an
    /// operation that asks before it acts can put its question on the screen instead of
    /// answering "run it again with --yes" to a reader who has no command line to add it to
    /// (interface spec 7.3).
    pub fn dispatch_from_key(
        &mut self,
        method: Method,
        client: Option<ClientId>,
    ) -> Result<serde_json::Value, ApiError> {
        self.dispatch_from(method, client, true)
    }

    fn dispatch_from(
        &mut self,
        method: Method,
        client: Option<ClientId>,
        from_key: bool,
    ) -> Result<serde_json::Value, ApiError> {
        let done = self.dispatch_inner(method, client.clone(), from_key);
        // No reply travels with a key's job: nobody is waiting on a keystroke, and a
        // failure reaches that client's hint row instead (`Core::answer`).
        for job in done.jobs {
            self.start_job(job, None, client.clone());
        }
        done.result
    }

    /// Runs one method and reads everything the handler recorded back out of its `Ctx`.
    /// The jobs it queued are started by the caller, which is the half that knows whether
    /// anyone is waiting for their answer.
    fn dispatch_inner(
        &mut self,
        method: Method,
        client: Option<ClientId>,
        from_key: bool,
    ) -> Dispatched {
        let mut ctx = Ctx {
            model: &mut self.model,
            panes: &mut self.panes,
            clients: &mut self.clients,
            config: &mut self.config,
            deps: &self.deps,
            facts: &self.facts,
            agents: &mut self.agents,
            core_tx: &self.core_tx,
            socket_path: &self.socket_path,
            state_dir: &self.state_dir,
            started_at: &self.started_at,
            client,
            from_key,
            events: Vec::new(),
            stop_requested: false,
            view_dirty: false,
            pending_spawns: Vec::new(),
            pending_kills: Vec::new(),
            detach_clients: Vec::new(),
            release_respawn_blocks: false,
            jobs: Vec::new(),
            claims: &self.claims,
            defer_reply: false,
        };
        let result = api::dispatch(method, &mut ctx);
        let events = std::mem::take(&mut ctx.events);
        let spawns = std::mem::take(&mut ctx.pending_spawns);
        let kills = std::mem::take(&mut ctx.pending_kills);
        let detaches = std::mem::take(&mut ctx.detach_clients);
        let jobs = std::mem::take(&mut ctx.jobs);
        let stop = ctx.stop_requested;
        // Read out of `ctx` before it is dropped: the borrow of `self` ends with it.
        let view_dirty = ctx.view_dirty;
        let release_blocks = ctx.release_respawn_blocks;
        let deferred = ctx.defer_reply;
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
        Dispatched {
            result,
            jobs,
            deferred,
        }
    }

    /// Runs a job on a blocking task and sends what it found back as `JobFinished`. Nothing
    /// that shells out runs on the core task, however fast it is (decision record 0006).
    ///
    /// The job's claim is taken here rather than in the handler, so that taking one and
    /// having a job to release it are the same act: a handler that took a claim and then
    /// failed to queue its job would hold that slot number for the life of the server.
    /// Taking it here is still early enough, because this runs inside the message that
    /// dispatched the handler and the next call is a message of its own.
    fn start_job(&mut self, job: CoreJob, reply: Option<JobReply>, client: Option<ClientId>) {
        let claim = job.claim();
        if let Some(claim) = &claim {
            self.claims.insert(claim.clone());
        }
        let tx = self.core_tx.clone();
        let running = tokio::task::spawn_blocking(move || run_job(job));
        tokio::spawn(async move {
            // One message however the job ended. A job that panicked would otherwise leave
            // the caller waiting for ever, which is the one failure a deferred reply must
            // not have: the same reasoning as the fetch wrapper in `start_due_fetches`.
            let outcome = running.await.unwrap_or_else(|e| JobOutcome::Failed {
                message: format!("the job did not finish: {e}"),
                code: ErrorCode::Internal,
            });
            let _ = tx
                .send(CoreMsg::JobFinished {
                    outcome,
                    reply,
                    client,
                    claim,
                })
                .await;
        });
    }

    /// A finished job, back on the core task: the model changes here and the caller is
    /// answered here. One writer, as everywhere else.
    fn job_finished(
        &mut self,
        outcome: JobOutcome,
        reply: Option<JobReply>,
        client: Option<ClientId>,
        claim: Option<String>,
    ) {
        // Before the arm, and whatever the arm does with it: the job is over either way, and
        // a claim a failure kept would hold its slot number until the server stopped.
        if let Some(claim) = claim {
            self.claims.remove(&claim);
        }
        let result = match outcome {
            JobOutcome::Failed { message, code } => {
                // A refusal is a red pill, the same line a result is a green one (interface
                // spec 7.3, and the theme table's `red` for refusal pills). It does not
                // duplicate `Core::answer`'s hint: `Hint` is drawn only by `top_bar.rs` and
                // `Pill` only by `sidebar.rs` and `overlay.rs`, so the two share no pixels. The
                // hint is the top bar's answer to the key just pressed; the pill is the result
                // line in the sidebar's hint row and the overlay's footer. A caller with a
                // command line gets neither - it gets the error itself.
                //
                // Every job's failure reaches this arm, `project.add`'s included, and that is
                // deliberate: a refusal is a refusal whatever asked for it. **Successes are not
                // symmetric.** `project_read` sets no pill, because registering a project shows
                // its own answer - the project appears in the box, which is the response
                // (principle 8) - while a create's answer is a slot that takes seconds to
                // build and a line saying it worked. Two jobs, two different needs, so the
                // green half is set where it is earned rather than in this shared arm.
                self.set_pill(client.as_ref(), message.clone(), false);
                Err(ApiError {
                    code,
                    message,
                    data: None,
                })
            }
            JobOutcome::ProjectRead {
                root,
                default_branch,
                slots,
            } => self.project_read(root, default_branch, slots),
            JobOutcome::Created {
                project,
                slot,
                path,
                branch,
                base,
                setup,
            } => self.workspace_created(client.clone(), project, slot, path, branch, base, setup),
            JobOutcome::Cleared {
                workspace,
                name,
                base,
            } => self.workspace_cleared(client.clone(), workspace, name, base),
            JobOutcome::Deleted {
                workspace,
                name,
                branch,
            } => self.workspace_deleted(client.clone(), workspace, name, branch),
        };
        self.answer(result, reply, client);
    }

    /// `workspace.create`'s model change: the slot record, its first tab, the pane that tab
    /// runs, and the `worktree.conf` run lines typed into that pane.
    ///
    /// The worktree and the branch are already on disk when this runs, so nothing here can
    /// leave a half-made workspace: every step below is in the model, and the model is only
    /// touched once the work on disk has all worked.
    #[allow(clippy::too_many_arguments)]
    fn workspace_created(
        &mut self,
        client: Option<ClientId>,
        project: ProjectId,
        slot: u32,
        path: PathBuf,
        branch: String,
        base: String,
        setup: Option<Setup>,
    ) -> Result<serde_json::Value, ApiError> {
        // A failure here leaves the worktree on disk with no record pointing at it, and that
        // is deliberate: the one way `add_slot` refuses is a slot number the model already
        // holds, and removing the directory would then delete a workspace that is really
        // there rather than tidying up after this call.
        let (workspace, mut events) = self.model.add_slot(&project, slot, path.clone())?;
        let (_tab, pane, more) = self.model.create_tab(&workspace, path.clone())?;
        events.extend(more);
        self.pending_events.extend(events);
        // The pane goes through the same list every other new pane goes through, so it is
        // started, sized and drawn the way one from `pane.split` or `tab.create` is.
        self.apply_side_effects(vec![pane.clone()], Vec::new(), Vec::new());
        // Typed into the pane, not run behind its back: a slow setup is then watched in the
        // workspace it belongs to (`worktree_conf`'s own note on `run_lines`). `ran` counts
        // what was really typed, so a pane whose process did not start reports nothing ran
        // rather than a number nobody could see (principle 4).
        let mut ran = 0;
        if let (Some(setup), Some(runtime)) = (setup.as_ref(), self.panes.get_mut(&pane)) {
            for line in &setup.run {
                runtime.write(format!("{line}\n").as_bytes());
                ran += 1;
            }
        }
        let summary = setup.as_ref().map(|setup| {
            let mut summary = setup.applied.summary();
            summary.ran = ran;
            // A `worktree.conf` that asked for nothing summarises as the empty string. It is
            // still `Some`: the file is there and it did nothing, which is a different fact
            // from a project with no `worktree.conf` at all (principle 4).
            summary.to_string()
        });
        let handle = domux_core::model::WorkspaceHandle::Slot(slot).to_string();
        self.set_pill(client.as_ref(), format!("Created {handle}"), true);
        let tabs = self
            .model
            .workspace(&workspace)
            .map(|w| w.tabs.len())
            .unwrap_or_default();
        api::ok(domux_core::api::WorkspaceCreated {
            id: workspace,
            project,
            handle,
            path,
            branch,
            base,
            setup: summary,
            tabs,
        })
    }

    /// `workspace.clear`'s model change: the slot's exited agent records, and nothing else. Its
    /// record, its number, its name and its tabs are what a clear keeps, and the rest of the
    /// work all happened on disk.
    ///
    /// Those records go because a clear puts the slot back at its base, so the work the
    /// sessions that ended there were about is gone too (M3 plan assumption 32; the
    /// architecture spec says an exited record stays "until you dismiss it or clear the
    /// workspace").
    ///
    /// **Here rather than in `api::workspace::clear`**, which is where the plan put it. That
    /// handler only queues the job: it does not know yet whether the slot will be reset, and a
    /// clear the job refuses - a dirty tree without `--yes` is the common one - would have
    /// taken the records with it and left the work in place.
    ///
    /// The event still goes out, because a subscriber cannot see the disk: `workspace.cleared`
    /// carries the base the slot was put back at, which is the one thing about the reset that
    /// was decided rather than given.
    fn workspace_cleared(
        &mut self,
        client: Option<ClientId>,
        workspace: WorkspaceId,
        name: String,
        base: String,
    ) -> Result<serde_json::Value, ApiError> {
        let gone = self.dismiss_exited_agents_of(&workspace);
        self.pending_events.extend(gone);
        self.pending_events
            .push(Event::WorkspaceCleared { workspace, base });
        self.set_pill(client.as_ref(), format!("Cleared {name}"), true);
        self.view_dirty = true;
        api::ok(domux_core::api::Ack { ok: true })
    }

    /// Dismisses every record of a workspace whose session is over, as a clear does.
    ///
    /// **Exited only.** A clear keeps the workspace and its panes, so an agent running in the
    /// slot is still running, and taking its record would destroy the session id, the recap
    /// and the name a resume needs. The observer would then put a bare record in its place:
    /// a live, resumable session made unresumable because the reader reset a worktree. The
    /// spec's sentence is about how long an exited record lasts, not a licence over a live
    /// one. A delete is the other case and takes everything, because the workspace itself is
    /// gone.
    ///
    /// `Model::dismiss_agent` rather than a removal of its own: taking away an exited record
    /// is one operation, and this is a clear asking for it once per record.
    ///
    /// Of the two caches `forget_agent_caches` drops below, only the transcript is ever there
    /// on this route: the filter takes records whose session is over, and such a record holds
    /// no working word, for the reason `api::agent::dismiss` sets out. The word half is live
    /// on the other caller, `forget_agent_caches_of`, which a delete uses and which takes
    /// working records too.
    ///
    /// That choice leaves the filter and the removal as two guards over one rule, and the
    /// removal's is the stronger: `dismiss_agent` refuses a live record on its own, so
    /// deleting the filter here would not let one through. The filter is what says which
    /// records a clear is asking about, and it is what keeps the `Err` arm below unreached -
    /// without it every live agent in the slot would log a warning on every clear, which is a
    /// normal outcome reported as a failure.
    fn dismiss_exited_agents_of(&mut self, workspace: &WorkspaceId) -> Vec<Event> {
        let doomed: Vec<(AgentId, Option<PathBuf>)> = self
            .model
            .agents_in_workspace(workspace)
            .into_iter()
            .filter(|a| !a.state.is_live())
            .map(|a| (a.id.clone(), a.transcript_path.clone()))
            .collect();
        let mut events = Vec::new();
        for (id, transcript) in doomed {
            // Both of `dismiss_agent`'s refusals are unreachable here: every id came from the
            // model a line ago, and the filter already excluded a live record. Written as a
            // match rather than an `expect` so a clear cannot panic the core if that ever
            // stops being true.
            match self.model.dismiss_agent(&id) {
                Ok(dismissed) => events.extend(dismissed),
                Err(e) => tracing::warn!("clearing {workspace}: {id} was not dismissed: {e}"),
            }
            self.forget_agent_caches(&id, transcript.as_deref());
        }
        events
    }

    /// Drops the working word and the cached transcript keyed to a record that has just gone.
    /// `api::agent::dismiss` does the same for the record it removes and says why: an agent id
    /// is never reissued, so nothing will ever ask for either again, and the pool of working
    /// words is finite, so a word never released is a slot lost for the life of the server.
    fn forget_agent_caches(&mut self, agent: &AgentId, transcript: Option<&Path>) {
        self.agents.forget_record(agent, transcript);
    }

    /// The same for every record of a workspace that is about to go with it, which is what a
    /// delete takes. Called before the removal, so a removal that refuses would release a word
    /// a record still holds. Nothing observable turns on that: a working word is picked from
    /// the agent id and comes back the same on the next look (M3 plan assumption 34), and a
    /// recap is re-read from the transcript.
    fn forget_agent_caches_of(&mut self, workspace: &WorkspaceId) {
        let doomed: Vec<(AgentId, Option<PathBuf>)> = self
            .model
            .agents_in_workspace(workspace)
            .into_iter()
            .map(|a| (a.id.clone(), a.transcript_path.clone()))
            .collect();
        for (id, transcript) in doomed {
            self.forget_agent_caches(&id, transcript.as_deref());
        }
    }

    /// `workspace.delete`'s model change: the panes' processes, then the record.
    ///
    /// The worktree and the branch are already gone when this runs, so nothing here can leave
    /// a half-deleted workspace. `Model::remove_workspace` is what takes the record, and it
    /// retires the workspace id and every tab and pane id under it (architecture spec 5):
    /// nothing here retires anything itself, because one place owns that.
    ///
    /// The panes are collected before the record goes. `remove_workspace` takes the tabs and
    /// the panes with it without passing through `close_pane`, so nothing else would ever kill
    /// these PTYs and the processes would outlive the slot on screen - the same reasoning
    /// `api::project::remove` gives for a project. The agent records go the same way, inside
    /// `remove_workspace`; only their caches are dropped here, because the model cannot reach
    /// them.
    fn workspace_deleted(
        &mut self,
        client: Option<ClientId>,
        workspace: WorkspaceId,
        name: String,
        branch: String,
    ) -> Result<serde_json::Value, ApiError> {
        let doomed: Vec<PaneId> = self
            .model
            .workspace(&workspace)
            .map(|w| w.tabs.iter().flat_map(|t| t.layout.pane_ids()).collect())
            .unwrap_or_default();
        self.forget_agent_caches_of(&workspace);
        let (handle, events) = self.model.remove_workspace(&workspace)?;
        self.pending_events.extend(events);
        // A client that was in the slot is now pointing at a workspace the model does not
        // hold, which no frame and no view method can answer for (principle 2). Same rule and
        // same code as `project.remove`, which strands clients the same way.
        let moved = api::project::reseat_stranded_clients(&mut self.model);
        self.pending_events.extend(moved);
        self.apply_side_effects(Vec::new(), doomed, Vec::new());
        // The branch is named only when it is news, and when it is, it is named **instead of**
        // the workspace rather than beside it.
        //
        // A slot's branch is named after its handle, so for an untouched one naming both would
        // say `workspace-1` twice. When they differ the branch is the one thing the reader
        // could not have worked out, and on the shell path it may not even be the name the
        // question gave them, so it is what this line exists to carry.
        //
        // The name is what gives way, because the reader asked to delete that workspace and
        // already knows which. It has to give way to something: the narrowest hint row this
        // draws in is the sidebar's, `SIDEBAR_WIDTH` less two, and `Deleted {name} · {branch}`
        // ran past it and lost the branch to an ellipsis - defeating this line exactly when it
        // mattered. `Deleted {branch}` fits that row for a branch of 28 columns or less, and a
        // longer one is cut from its tail rather than removed whole, because it leads.
        // `the_hint_row_fits_a_branch_of_twenty_eight_columns_and_not_twenty_nine` holds the
        // boundary and `a_branch_too_long_for_the_hint_row_still_leads_the_result` holds the
        // leading, so neither number here is prose nobody checks.
        let said = if branch == handle.to_string() {
            format!("Deleted {name}")
        } else {
            format!("Deleted {branch}")
        };
        self.set_pill(client.as_ref(), said, true);
        api::ok(domux_core::api::Ack { ok: true })
    }

    /// Puts one line of result in a client's hint row or footer (interface spec 7.3 and 12.12),
    /// stamped from the core's clock, which is the same clock `tick` measures its age against.
    ///
    /// This is the pill for an answer that arrives back on the core task: a finished job, or a
    /// key whose result `input` reads. `Ctx::set_pill` is the same pill from inside a handler,
    /// and both go through `core::set_pill` so the two cannot come to disagree about where a
    /// pill lands or when the view has changed.
    pub fn set_pill(&mut self, client: Option<&ClientId>, text: String, ok: bool) {
        let at = self.deps.clock.now().to_rfc3339();
        if set_pill(&mut self.model, client, text, ok, &at) {
            self.view_dirty = true;
        }
    }

    /// Drops every pill that has been showing for longer than `PILL_SECONDS`, so a result
    /// does not sit in the hint row for the rest of the session (interface spec 12.12).
    /// A pill whose stamp will not parse is dropped too, rather than kept for ever.
    fn expire_pills(&mut self) -> bool {
        let now = self.deps.clock.now();
        let mut cleared = false;
        for view in &mut self.model.clients {
            let Some(pill) = &view.pill else { continue };
            let age = chrono::DateTime::parse_from_rfc3339(&pill.at)
                .map(|at| now.signed_duration_since(at).num_seconds())
                .unwrap_or(i64::MAX);
            if age >= PILL_SECONDS as i64 {
                view.pill = None;
                cleared = true;
            }
        }
        cleared
    }

    /// `project.add`'s model change: register the path and adopt the worktrees the job
    /// found beside it.
    fn project_read(
        &mut self,
        root: PathBuf,
        default_branch: Option<String>,
        slots: Vec<(u32, PathBuf)>,
    ) -> Result<serde_json::Value, ApiError> {
        // Idempotence is decided here rather than in the handler, because the canonical
        // path is only known once the job has resolved it. A path that is already a project
        // is answered for as it stands, and nothing new is adopted: `project.add` is how a
        // path is registered, and `workspace.create` is how a slot is made.
        if let Some(existing) = self.model.project_at(&root) {
            return api::project::added(existing, Vec::new());
        }
        let added = match &default_branch {
            Some(branch) => self.model.add_git_project(root.clone(), branch.clone()),
            None => self.model.add_folder_project(root.clone()),
        };
        // `Core::handle` returns nothing, so a failure here is answered rather than
        // propagated with `?` to a caller that is not there: the one who is waiting is on
        // the other end of `reply`.
        let (project, _main, mut events) = added?;
        // `add_folder_project` is M1's and reports no events, because `project.added` is an
        // M2 event. `add_git_project` is M2's and reports it itself, so only the folder
        // branch pushes one.
        if default_branch.is_none() {
            let name = self
                .model
                .project(&project)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            events.push(Event::ProjectAdded {
                project: project.clone(),
                name,
                root: root.clone(),
            });
        }
        let mut adopted = Vec::new();
        for (slot, slot_path) in slots {
            match self.model.add_slot(&project, slot, slot_path) {
                Ok((_, more)) => {
                    adopted.push(format!("workspace-{slot}"));
                    events.extend(more);
                }
                // A slot the model would not take is left out of `adopted` rather than
                // named in it: the answer says what is registered, not what was on disk.
                Err(e) => tracing::warn!(
                    "could not adopt workspace-{slot} in {}: {}",
                    root.display(),
                    e.message
                ),
            }
        }
        self.pending_events.extend(events);
        // Every new workspace gets its tab and its shell, the same invariant
        // `apply_side_effects` keeps for every other path that makes one.
        self.apply_side_effects(Vec::new(), Vec::new(), Vec::new());
        // Belt, and deliberately unpinned, for the reason `api::project::remove`'s copy of
        // this line gives: `apply_side_effects` has just spawned a pane for every workspace
        // this registered, so it has already marked the view and no test can tell this line
        // from its absence.
        self.view_dirty = true;
        let registered = self.model.project(&project).ok_or_else(|| {
            ApiError::internal("the project was registered and is not there any more")
        })?;
        api::project::added(registered, adopted)
    }

    /// Sends a job's answer where it belongs: to the caller waiting on it, or, for a job a
    /// key started, to that client's hint row, which is where a failed key's message goes
    /// (principle 8).
    fn answer(
        &mut self,
        result: Result<serde_json::Value, ApiError>,
        reply: Option<JobReply>,
        client: Option<ClientId>,
    ) {
        match reply {
            Some(JobReply { id, tx }) => {
                let _ = tx.send(match result {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(id, e),
                });
            }
            None => {
                let Err(e) = result else { return };
                tracing::info!("{}", e.message);
                let mut told = false;
                if let Some(conn) = client.and_then(|c| self.clients.get_mut(&c)) {
                    conn.hint = Some(Hint::action(e.message));
                    told = true;
                }
                self.view_dirty |= told;
            }
        }
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
            // Every close reaches this list: `close_pane`, `pane.close`, `tab.close`, a
            // deleted workspace and a removed project all put their panes here, and the last
            // three never pass through `close_pane` at all. So this is where a pane going
            // away ends the agents that were running in it, rather than in each of those.
            self.agents_gone_with_pane(&pane);
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
        let area = render::tab_workpanel(&self.model, &loc.tab, fallback);
        domux_core::model::layout::solve(&tab.layout, area, tab.zoomed.as_ref())
            .into_iter()
            .find(|(p, _)| p == pane)
            .map(|(_, r)| Size {
                cols: r.width.saturating_sub(2).max(1),
                rows: r.height.saturating_sub(2).max(1),
            })
            .unwrap_or(fallback)
    }

    /// Publishes what the observer did and answers whether it did anything. A record that
    /// stopped working frees its working word, exactly as a hook report does: the word is per
    /// working agent, and an agent that exited is not one.
    fn agents_changed(&mut self, events: Vec<Event>) -> bool {
        if events.is_empty() {
            return false;
        }
        self.agents.release_words_of(&events);
        self.pending_events.extend(events);
        true
    }

    /// Every live record on `pane` exits now: a record must never read `working` on a pane
    /// that is gone. The records are found by the pane they hold, so this answers whether or
    /// not the model still has that pane, and each record keeps the workspace it started in.
    fn agents_gone_with_pane(&mut self, pane: &PaneId) {
        let now = self.deps.clock.now().to_rfc3339();
        let events = observer::pane_gone(&mut self.model, pane, &now);
        if self.agents_changed(events) {
            self.view_dirty = true;
        }
    }

    /// What the process inspector and the emulator say about one pane right now. Each field
    /// is `None` when nothing answered, so `set_pane_facts` leaves that fact alone rather
    /// than clearing it.
    ///
    /// The foreground process comes back beside the facts because the observer asks a second
    /// question of the same answer: the pane box wants the command's name, and the observer
    /// wants to know whether that command is a known agent.
    fn observe_pane(&self, pane: &PaneRuntime) -> (PaneFacts, Option<ForegroundProcess>) {
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
        (facts, fg)
    }

    /// One frame on, and a redraw, but only while there is something for the glyph to report
    /// on (principle 7). The counter is the core's, so every client on this server draws the
    /// same frame of the same animation.
    ///
    /// The ticker that sends this runs from the first message to the last and is never
    /// started or stopped (M3 plan assumption 35). A timer that starts and stops is state,
    /// and state can be wrong: a start that is missed leaves a working agent with a glyph
    /// that never turns, and a stop that is missed is the redraw this guard is here to
    /// prevent, left running for the life of the server. The guard is one boolean walk over
    /// the records, twelve and a half times a second, and it cannot fail.
    fn animation_tick(&mut self) {
        if crate::agents::observer::any_working(&self.model) {
            self.agents.glyph_tick = self.agents.glyph_tick.wrapping_add(1);
            self.view_dirty = true;
        }
    }

    fn tick(&mut self) {
        let mut changed = false;
        let mut seen: Vec<PaneProcess> = Vec::with_capacity(self.panes.len());
        for (id, pane) in &self.panes {
            let (facts, foreground) = self.observe_pane(pane);
            if let Some(current) = self.model.pane(id) {
                if (facts.command.is_some() && facts.command != current.command)
                    || (facts.cwd.is_some() && facts.cwd.as_ref() != Some(&current.cwd))
                    || (facts.title.is_some() && facts.title != current.title)
                {
                    changed = true;
                }
            }
            seen.push(PaneProcess {
                pane: id.clone(),
                foreground,
            });
            self.model.set_pane_facts(id, facts);
        }
        // The agent question, off the same walk: the inspector was asked once and both the
        // pane boxes and the records read that one answer.
        let now = self.deps.clock.now().to_rfc3339();
        let events = observer::run(
            &mut self.model,
            &seen,
            &self.agents.manifests,
            self.deps.inspector.as_ref(),
            &now,
        );
        changed |= self.agents_changed(events);
        let minute = self.deps.clock.now().format("%H:%M").to_string();
        if self.last_minute.as_ref() != Some(&minute) {
            self.last_minute = Some(minute);
            changed = true;
        }
        // Not folded into `changed`: an expired pill changes what is drawn but nothing that
        // is persisted, and `changed` is what asks for a write of the state file too.
        if self.expire_pills() {
            self.view_dirty = true;
        }
        if changed {
            self.view_dirty = true;
            self.persist();
        }
        self.start_due_fetches();
    }

    /// Starts every provider whose interval has passed, each on its own blocking task. A
    /// provider shells out to git or to `gh` and the core cannot wait on either: one slow
    /// `gh` inside the core task stops every frame in every pane. The answer comes back as
    /// `FactFetched` like any other message.
    fn start_due_fetches(&mut self) {
        self.facts.forget_deleted(&self.model);
        let now = self.deps.clock.now();
        // What domux observed an hour ago is not what is true now. A fact past its time to
        // live goes, and the screen is told, rather than an old pull request state being
        // drawn like a current one.
        for key in self.facts.expire(now) {
            self.pending_events.push(Event::FactUpdated {
                key,
                present: false,
            });
            self.view_dirty = true;
        }
        for (provider, target) in self.facts.due(&self.model, now) {
            let tx = self.core_tx.clone();
            let key = target.key.clone();
            let named = key.clone();
            let fetch = tokio::task::spawn_blocking(move || match provider.fetch(&target) {
                Ok(fact) => fact,
                // An error and an absence render the same, so only the log tells them
                // apart. Nothing is guessed in either case.
                Err(reason) => {
                    tracing::debug!("{named} is absent: {reason}");
                    None
                }
            });
            // One message for every fetch, whatever happened inside it. A provider that
            // panics would otherwise leave its key in flight for the life of the server,
            // and its last value on the screen with nothing left to replace it.
            tokio::spawn(async move {
                let fact = match fetch.await {
                    Ok(fact) => fact,
                    Err(e) => {
                        tracing::warn!("the provider for {key} did not finish: {e}");
                        None
                    }
                };
                let _ = tx.send(CoreMsg::FactFetched { key, fact }).await;
            });
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
        // Sizes and facts before the model, for the same reason one step smaller: whatever
        // this batch changed about a pane's size or a target's fact is already published by
        // the time a reader sees the model, rather than a reader seeing this batch's model
        // and a size or a fact still one batch behind. This is not "a fact for every target":
        // a fact is present or it is absent (`facts/mod.rs`'s `FactRegistry` doc comment), and
        // a target with no answer yet, or whose answer expired, has no entry here either.
        *self.pane_sizes.lock().unwrap() = self
            .panes
            .iter()
            .map(|(id, p)| (id.clone(), p.size()))
            .collect();
        *self.published_facts.lock().unwrap() = self.facts.all();
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
        // alone: a resize would churn its program for no visible result. `tab_workpanel` still
        // raises an absent current size to the minimum, rather than adopting a tiny screen. The
        // rectangle itself comes from `render::tab_workpanel`, the same function the
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
            let area = render::tab_workpanel(&self.model, &tab_id, fallback);
            let Some(tab) = self.model.tab(&tab_id) else {
                continue;
            };
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
        // One view for every client on this server: the agent list is the same list
        // wherever it is drawn, and the glyph is the core's frame, not each client's.
        let now = self.deps.clock.now();
        let agents = agents_view(&self.model, &mut self.agents, &self.config.keymap, now);
        for view in self.model.clients.clone() {
            let Some(conn) = self.clients.get_mut(&view.id) else {
                continue;
            };
            let input = RenderInput {
                model: &self.model,
                facts: &self.facts,
                panes: &self.panes,
                agents: &agents,
                view: &view,
                keymap: &self.config.keymap,
                now: self.deps.clock.now(),
                config_error: self.config.error.as_ref(),
                hint: conn.hint.as_ref(),
                notes: &self.notes,
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

/// One job, on a blocking task. Every arm here shells out or touches the filesystem, which
/// is the whole reason the lane exists (decision record 0006).
fn run_job(job: CoreJob) -> JobOutcome {
    match job {
        CoreJob::ReadProject { path } => read_project(&path),
        CoreJob::CreateWorkspace {
            project,
            root,
            slot,
            path,
            branch,
            base,
        } => create_workspace(project, root, slot, path, branch, base),
        CoreJob::ClearWorkspace {
            workspace,
            name,
            root,
            path,
            base,
            yes,
        } => clear_workspace(workspace, name, &root, &path, base, yes),
        CoreJob::DeleteWorkspace {
            workspace,
            name,
            root,
            path,
            expected_branch,
            base,
            force,
        } => delete_workspace(workspace, name, &root, &path, expected_branch, base, force),
    }
}

/// A git command that did not work, as an outcome. `unavailable` rather than `internal`:
/// git ran and said no, or could not be started, and neither is a fault in domux's own
/// reasoning.
fn unavailable(e: crate::git::GitError) -> JobOutcome {
    JobOutcome::Failed {
        message: e.to_string(),
        code: ErrorCode::Unavailable,
    }
}

/// V1's `resetGitWorkspace`: the slot's branch back at the base, and every untracked file
/// gone. The slot, its number, its name, its record and its tabs are untouched.
///
/// The dirty gate is here rather than in the handler because only `git::is_dirty` can answer
/// it and it shells out. Every step that can refuse runs before anything is thrown away.
///
/// **The branch is read here, not taken from the branch fact**, and both jobs do it.
///
/// A slot's handle is `workspace-1` and the branch it was made on has the same name, but
/// nothing stops the author checking out `feat/auth-cleanup` in it, so the handle cannot be
/// used at all: acting on it would reset or delete a branch the worktree is not on and report
/// that it had done the right thing (principle 4).
///
/// The fact cannot be used either, and the reason is not that it would make the guard and the
/// action disagree with each other. It would not: one name would go to both `git::is_dirty`
/// and the command that destroys, so they would agree, and be wrong together. The reason is
/// that the fact can name a branch the worktree has since left. It carries a 30 second time to
/// live, so it is already up to half a minute old when the question is composed, and from a
/// shell the reader then types `--yes` whenever they get to it. **Neither window needs the
/// author to do anything while the question is up.** A clear is the sharper case:
/// `git::reset_to_base` runs `git checkout -q <branch>` in the worktree, so a stale name would
/// move the slot onto another branch and hard-reset that one, quietly.
///
/// The confirmation still names the fact, because it is composed on the core task where no git
/// command may run. `CoreJob::DeleteWorkspace::expected_branch` carries the question's answer
/// into the job so it can refuse when the worktree has moved, which holds for a key and not for
/// a shell: see that field for why, and for what it costs.
fn clear_workspace(
    workspace: WorkspaceId,
    name: String,
    root: &Path,
    path: &Path,
    base: Option<String>,
    yes: bool,
) -> JobOutcome {
    let branch = match crate::git::branch_of(path) {
        Ok(branch) => branch,
        Err(e) => return unavailable(e),
    };
    let base = crate::git::base_ref(root, base.as_deref());
    if !yes {
        match crate::git::is_dirty(path, &branch, &base) {
            Ok(true) => {
                return JobOutcome::Failed {
                    message: format!(
                        "{name} has uncommitted or unpushed changes; clear it with --yes to throw them away"
                    ),
                    code: ErrorCode::Refused,
                }
            }
            Ok(false) => {}
            // Not "there is nothing to lose". A guard that could not run has not passed
            // (principle 4), and the answer it was guarding throws work away.
            Err(e) => return unavailable(e),
        }
    }
    if let Err(e) = crate::git::reset_to_base(path, &branch, &base) {
        return unavailable(e);
    }
    if let Err(e) = crate::git::clean(path) {
        return unavailable(e);
    }
    JobOutcome::Cleared {
        workspace,
        name,
        base,
    }
}

/// The worktree and the local branch of one slot, in that order, and nothing until every
/// guard has passed.
///
/// `git worktree remove` refuses a worktree holding modified or untracked files on its own,
/// and `force` turns that off. What it never sees is a slot holding a week of commits that
/// were never pushed: those go with the branch and nothing brings them back, so
/// `git::is_dirty` in front of it is the only guard against that (`git.rs` says the same at
/// the declaration). Both are behind the one `force` the caller passed, so a reader who said
/// "delete it anyway" says it once.
fn delete_workspace(
    workspace: WorkspaceId,
    name: String,
    root: &Path,
    path: &Path,
    expected_branch: Option<String>,
    base: Option<String>,
    force: bool,
) -> JobOutcome {
    // Read, not taken from the fact: `clear_workspace` above says why, and the reason is the
    // same one both jobs turn on.
    let branch = match crate::git::branch_of(path) {
        Ok(branch) => branch,
        Err(e) => return unavailable(e),
    };
    // The question named a branch and the worktree is on another one, so the consent that was
    // given is not consent to this. Refusing names both and leaves everything where it is; the
    // reader asks again and answers the truth. `conflict` is the code for exactly this: the
    // state changed underneath the caller.
    if let Some(expected) = expected_branch {
        if expected != branch {
            return JobOutcome::Failed {
                message: format!(
                    "{name} is on {branch} now, not {expected}, so nothing was removed; ask again"
                ),
                code: ErrorCode::Conflict,
            };
        }
    }
    if !force {
        let base = crate::git::base_ref(root, base.as_deref());
        match crate::git::is_dirty(path, &branch, &base) {
            Ok(true) => {
                return JobOutcome::Failed {
                    message: format!(
                        "{name} has uncommitted or unpushed changes; delete it with --force or commit and push first"
                    ),
                    code: ErrorCode::Refused,
                }
            }
            Ok(false) => {}
            // Same reasoning as the clear above, and it matters more here: the answer this
            // guards is a worktree and a branch that nothing brings back.
            Err(e) => return unavailable(e),
        }
    }
    if let Err(e) = crate::git::worktree_remove(root, path, &branch, force) {
        return unavailable(e);
    }
    JobOutcome::Deleted {
        workspace,
        name,
        branch,
    }
}

/// V1's `provisionWorkspace`, in its order: the worktree on a fresh branch from the base,
/// then the project's `worktree.conf` read, parsed and applied.
///
/// Everything here forks or touches the filesystem, and `git worktree add` fetches from a
/// remote, which is the whole reason the create is a job rather than a handler.
///
/// A failure adds nothing to the model, so there is never a half-made workspace record. What
/// it could leave is a half-made worktree, and it does not: once `worktree_add` has worked,
/// every failure below takes the worktree and its branch back out before it reports. The
/// report is the original failure, because that is the one that says what to fix; a rollback
/// that fails as well is named beside it, because a directory the server could not remove is
/// something the author has to know about.
fn create_workspace(
    project: ProjectId,
    root: PathBuf,
    slot: u32,
    path: PathBuf,
    branch: String,
    base: Option<String>,
) -> JobOutcome {
    let base = crate::git::base_ref(&root, base.as_deref());
    if let Err(e) = crate::git::worktree_add(&root, &path, &branch, &base) {
        return JobOutcome::Failed {
            message: e.to_string(),
            code: ErrorCode::Unavailable,
        };
    }
    let conf = root.join(worktree_conf::CONF_PATH);
    let text = match std::fs::read_to_string(&conf) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        // A setup file that is there and cannot be read is not a project without one:
        // carrying on would build a slot missing the files its `worktree.conf` names and
        // report that as a success (principle 4).
        Err(e) => {
            return rolled_back(
                &root,
                &path,
                &branch,
                format!("could not read {}: {e}", conf.display()),
                ErrorCode::Internal,
            )
        }
    };
    let setup = text.map(|text| {
        let (directives, warnings) = worktree_conf::parse(&text);
        for warning in warnings {
            tracing::warn!("{}: {warning}", conf.display());
        }
        let applied = worktree_conf::apply(&root, &path, &directives);
        for failure in applied.failures() {
            tracing::warn!("{}: {failure}", conf.display());
        }
        Setup {
            run: worktree_conf::run_lines(&directives),
            applied,
        }
    });
    JobOutcome::Created {
        project,
        slot,
        path,
        branch,
        base,
        setup,
    }
}

/// Takes a worktree this job made back out and reports `message`, naming the rollback's own
/// failure beside it when the directory would not go.
fn rolled_back(
    root: &Path,
    path: &Path,
    branch: &str,
    message: String,
    code: ErrorCode,
) -> JobOutcome {
    match crate::git::worktree_remove(root, path, branch, true) {
        Ok(()) => JobOutcome::Failed { message, code },
        Err(e) => JobOutcome::Failed {
            message: format!("{message}; and {} is still there: {e}", path.display()),
            code,
        },
    }
}

/// What `project.add` needs to know about a path: whether it is there, whether it is a
/// repository, what `origin/HEAD` points at, and which slot directories exist beside it.
///
/// Three forks (`git rev-parse`, `git symbolic-ref`, `git worktree list`) and a handful of
/// syscalls. The syscalls could run on the core task; they are here so one place owns the
/// whole answer and the handler has one thing to queue.
fn read_project(path: &str) -> JobOutcome {
    let root = match std::fs::canonicalize(path) {
        Ok(root) => root,
        Err(_) => {
            return JobOutcome::Failed {
                message: format!("{path} does not exist"),
                code: ErrorCode::NotFound,
            }
        }
    };
    if !root.is_dir() {
        return JobOutcome::Failed {
            message: format!(
                "{} is a file; name the folder that holds the project",
                root.display()
            ),
            code: ErrorCode::InvalidParams,
        };
    }
    // A plain folder is a project with `main` and nothing else. `default_branch` answers
    // `main` for a directory that is not a repository at all, so the question has to be
    // asked separately rather than read out of its answer.
    if !crate::git::is_repo(&root) {
        return JobOutcome::ProjectRead {
            root,
            default_branch: None,
            slots: Vec::new(),
        };
    }
    let default_branch = crate::git::default_branch(&root);
    let slots = match crate::git::existing_slots(&root) {
        Ok(slots) => slots,
        // A worktree directory that is there and cannot be read is an error, not "no
        // slots": adopting nothing would hand out a slot number that is already taken.
        Err(e) => {
            return JobOutcome::Failed {
                message: e.to_string(),
                code: ErrorCode::Internal,
            }
        }
    };
    let slots = slots
        .into_iter()
        .map(|slot| (slot, slot_directory(&root, slot)))
        .collect();
    JobOutcome::ProjectRead {
        root,
        default_branch: Some(default_branch),
        slots,
    }
}

/// Where a slot really is: under `.domux/worktrees` when that directory is there, and under
/// the name V1 used before the rename when it is not. `existing_slots` reads both, so a
/// project the author has been using with V1 adopts its worktrees at the paths they are at
/// rather than at the paths V2 would have made.
fn slot_directory(root: &Path, slot: u32) -> PathBuf {
    let current = crate::git::slot_path(root, slot);
    if current.is_dir() {
        return current;
    }
    root.join(crate::git::LEGACY_WORKTREE_DIR)
        .join(crate::git::slot_branch(slot))
}

/// One `AgentsView`: the records in sort order, their places, and this frame's glyph and
/// working words.
///
/// Free rather than a method on `Core`, because two callers need it. `Core::render` builds one
/// per frame, and `api::list` builds one to walk the cursor over exactly the rows that frame
/// drew (interface spec 12.2). A second builder would be a second list, and the cursor would
/// come to rest on rows the box does not have.
///
/// Building it here is what keeps `render` free of the Model's lookups, and what makes the
/// sidebar and the agents overlay draw one agent one way.
pub(crate) fn agents_view(
    model: &Model,
    state: &mut crate::agents::AgentsState,
    keymap: &domux_core::keymap::Keymap,
    now: DateTime<Local>,
) -> crate::render::agents_box::AgentsView {
    use crate::render::agents_box::{AgentEntry, AgentsView};
    let glyph = crate::agents::labels::frame_at(state.glyph_tick);
    let red_dots = model.red_dot_count();
    let sorted = model.sorted_agents();
    let mut entries = Vec::with_capacity(sorted.len());
    for a in sorted {
        // A working word for a working agent and for no other, so a row that must not carry
        // one cannot (principle 4).
        let word = if a.state == AgentState::Working {
            state.words.word_for(&a.id)
        } else {
            ""
        };
        entries.push(AgentEntry {
            id: a.id.clone(),
            kind: a.kind,
            name: a.name.clone(),
            state: a.state,
            unseen: a.unseen,
            recap: a.recap.clone(),
            place_with_tab: crate::agents::context::place_of(model, a),
            place_without_tab: crate::agents::context::place_without_tab(model, a),
            last_activity_at: a.last_activity_at.clone(),
            word,
        });
    }
    AgentsView {
        agents: entries,
        glyph,
        now,
        red_dots,
        // One lookup a frame, so an exited row and the sidebar's hint row name the same key
        // for one action (principle 3).
        resume_key: keymap.list_key_for(crate::render::agents_box::RESUME_ACTION),
    }
}

/// Puts one line of result in a client's hint row or footer, green when it worked and red when
/// it did not (interface spec 7.3 and 12.12). True when a client took it, which is when the view
/// changed.
///
/// A rule over the model rather than a method, because a pill is set from two sides of the same
/// message: a handler, which holds the model through its `Ctx` and is gone before the next
/// message arrives, and the core, which sets one for a finished job or a key's refusal.
/// `Ctx::set_pill` and `Core::set_pill` are the two wrappers, and each supplies the clock it
/// has.
///
/// A call with no client draws nothing, and that is the honest outcome: a pill is a place on a
/// screen, and a caller with no screen has already been answered by its reply.
pub(crate) fn set_pill(
    model: &mut Model,
    client: Option<&ClientId>,
    text: String,
    ok: bool,
    at: &str,
) -> bool {
    let Some(view) = client.and_then(|c| model.client_mut(c)) else {
        return false;
    };
    view.pill = Some(Pill {
        text,
        ok,
        at: at.to_string(),
    });
    true
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
        | ListDown(p) | ListUp(p) | ListActivate(p) | ListFilter(p) | AgentsOpen(p)
        | AgentsClose(p) | FocusNextRegion(p) => p.client.clone(),
        AgentGet(p) | AgentFocus(p) | AgentDismiss(p) => p.client.clone(),
        AgentResume(p) => p.client.clone(),
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
        | WorkspaceResume(_)
        | AgentList(_)
        | AgentSelf(_)
        | AgentReport(_)
        | AgentSend(_)
        | AgentRead(_)
        | AgentWait(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::FakeSpawner;
    use crate::process::FakeInspector;
    use crate::{load_config, FixedClock};
    use domux_core::api::NoParams;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn core(dir: &Path) -> Core {
        core_with_providers(dir, Vec::new()).0
    }

    /// The core and the receiving end of its own channel, which the fact scheduler sends to.
    fn core_with_providers(
        dir: &Path,
        providers: Vec<Arc<dyn crate::facts::FactProvider>>,
    ) -> (Core, mpsc::Receiver<CoreMsg>) {
        core_with(dir, providers, &dir.join("missing.json"))
    }

    /// The same, over a state file that is there, for a test about what a start makes of one.
    fn core_with(
        dir: &Path,
        providers: Vec<Arc<dyn crate::facts::FactProvider>>,
        state_file: &Path,
    ) -> (Core, mpsc::Receiver<CoreMsg>) {
        let project = dir.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let (core_tx, core_rx) = mpsc::channel(8);
        let (persist_tx, _persist_rx) = mpsc::channel(8);
        let opts = ServerOptions {
            socket_path: dir.join("s.sock"),
            state_dir: dir.join("state"),
            config: load_config(&dir.join("none.toml")),
            project_root: project,
            providers,
            deps: CoreDeps {
                spawner: Arc::new(FakeSpawner::default()),
                inspector: Arc::new(FakeInspector::default()),
                clock: Arc::new(FixedClock::at("2026-09-04T14:32:00")),
                id_seed: 7,
            },
        };
        let core = Core::new(
            opts,
            core_tx,
            persist_tx,
            state_file,
            Arc::new(Mutex::new(Model::new(7))),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
        )
        .unwrap();
        (core, core_rx)
    }

    /// One attached client, without a socket. `attach` is what builds a `ClientView`, so a
    /// test that hand-built one would be testing its own copy of the shape.
    fn attached(core: &mut Core) -> ClientId {
        let (tx, rx) = mpsc::channel(8);
        // The receiver outlives the call: `attach` sends `Welcome`, and a closed channel
        // would make that send fail silently and the test read a state it did not set up.
        let id = core
            .attach(
                domux_core::proto::Hello {
                    version: domux_core::VERSION.into(),
                    protocol: domux_core::proto::PROTOCOL_VERSION,
                    cols: 80,
                    rows: 24,
                    caps: Default::default(),
                },
                tx,
            )
            .expect("attach");
        drop(rx);
        id
    }

    /// A finished job releases the claim it held and **only** that one.
    ///
    /// A unit test rather than a harness one, because no fixture the harness can build tells
    /// the two apart. `clear()` and `remove()` differ only when a third create arrives while
    /// two are still building: with two in flight, the second has already chosen its number by
    /// the time the first finishes, so clearing everything is harmless. Reaching the
    /// difference end to end needs a fourth call landing inside the window between one job
    /// finishing and the others ending - a race, and a test that is a race is a test that
    /// inverts a mutation verdict when it flakes. Here the state is set directly and the
    /// question is asked exactly.
    ///
    /// This is sufficient rather than a second best. The bridge from the invariant to the
    /// consequence is tested link by link elsewhere: that a handler reads the claims and skips
    /// what they hold (`a_slot_number_another_call_has_spoken_for_is_skipped`), that a job in
    /// flight really holds its number
    /// (`two_creates_in_flight_at_once_take_two_different_slot_numbers`), and that a number
    /// comes back when its job ends (`a_create_that_failed_gives_its_slot_number_back`). The
    /// one link no test exercises is arrival timing, and it carries no logic: `HashSet::remove`
    /// does not behave differently at three keys than at two.
    #[test]
    fn a_finished_job_releases_its_own_claim_and_leaves_the_others_held() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let (first, second) = (
            slot_claim(&ProjectId("pr_1".into()), 1),
            slot_claim(&ProjectId("pr_1".into()), 2),
        );
        core.claims.insert(first.clone());
        core.claims.insert(second.clone());

        core.job_finished(
            JobOutcome::Failed {
                message: "no".into(),
                code: ErrorCode::Unavailable,
            },
            None,
            None,
            Some(first.clone()),
        );

        assert!(!core.claims.contains(&first), "its own claim is released");
        assert!(
            core.claims.contains(&second),
            "and a create still building keeps its number; releasing everything here would \
             hand workspace-2 to the next call while this one is still making it"
        );
    }

    /// Two projects can hold the same slot number at once, so a create in one never moves the
    /// numbering of another.
    ///
    /// The claim key carries the project for this reason. Without it both projects share one
    /// key, and a second project's first slot comes out numbered 2 - permanently, since a slot
    /// number never renumbers (architecture spec 2). Every harness fixture has one project, so
    /// this is the shape that separates them.
    #[test]
    fn a_claim_in_one_project_does_not_move_another_project_s_numbering() {
        let mut model = Model::new(7);
        let (one, _, _) = model
            .add_git_project(PathBuf::from("/a/audrey-app"), "main".into())
            .unwrap();
        let (two, _, _) = model
            .add_git_project(PathBuf::from("/b/other-app"), "main".into())
            .unwrap();
        let mut claims = HashSet::new();
        claims.insert(slot_claim(&one, 1));

        let free = |project: &ProjectId| {
            model
                .lowest_free_slot(project, |n| claims.contains(&slot_claim(project, n)))
                .unwrap()
        };
        assert_eq!(free(&one), 2, "the project holding the claim skips it");
        assert_eq!(
            free(&two),
            1,
            "and the other project's first slot is still 1"
        );
    }

    /// A pill stays for `PILL_SECONDS` and then goes, so a result does not sit in the hint
    /// row for the rest of the session (interface spec 12.12).
    ///
    /// The clock is fixed, so the stamp is what moves. Both sides of the boundary are here:
    /// a pill one second short of the limit is still showing, which is what tells a working
    /// expiry from one that clears every pill on the first tick, and the second half of the
    /// test would pass against that.
    ///
    /// The screen is asserted at both ends, and `view_dirty` is cleared before each so the
    /// last thing that set it cannot stand in for the thing under test. The harness cannot
    /// carry either claim: a create marks the view through `apply_side_effects` as well, and
    /// the first `tick` of a server marks it because the minute has changed.
    #[test]
    fn a_pill_goes_after_six_seconds_and_not_before() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let client = attached(&mut core);
        core.view_dirty = false;
        core.set_pill(Some(&client), "Created workspace-1".into(), true);
        assert!(
            core.view_dirty,
            "a pill nothing drew is a result nobody sees"
        );
        let now = core.deps.clock.now();
        let stamp = |core: &mut Core, seconds: i64| {
            let at = (now - chrono::Duration::seconds(seconds)).to_rfc3339();
            core.model
                .client_mut(&client)
                .unwrap()
                .pill
                .as_mut()
                .unwrap()
                .at = at;
        };

        stamp(&mut core, PILL_SECONDS as i64 - 1);
        core.tick();
        assert_eq!(
            core.model
                .client(&client)
                .and_then(|v| v.pill.as_ref())
                .map(|p| p.text.as_str()),
            Some("Created workspace-1"),
            "a pill that has not run out is still showing"
        );

        stamp(&mut core, PILL_SECONDS as i64);
        core.view_dirty = false;
        core.tick();
        assert!(
            core.model.client(&client).unwrap().pill.is_none(),
            "and one that has run out is gone"
        );
        assert!(
            core.view_dirty,
            "the screen is told, or the row stays drawn"
        );
    }

    /// A pill lands on the client the call came from, and every stale pill goes, not just the
    /// first one.
    ///
    /// Two clients, because with one attached "the caller's screen" and "some screen" are the
    /// same screen, and "every pill expires" and "the first pill expires" are the same
    /// sentence. The second client is also the one the call names, so the fixture puts the
    /// answer somewhere the wrong implementation would not look.
    #[test]
    fn a_pill_lands_on_the_calling_client_and_every_stale_pill_expires() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let first = attached(&mut core);
        let second = attached(&mut core);
        assert_ne!(first, second);

        core.set_pill(Some(&second), "Created workspace-1".into(), true);
        assert!(
            core.model.client(&first).unwrap().pill.is_none(),
            "not the first client just because it is first"
        );
        assert_eq!(
            core.model
                .client(&second)
                .and_then(|v| v.pill.as_ref())
                .map(|p| p.text.as_str()),
            Some("Created workspace-1")
        );

        // Both stale, so an expiry that stops after the first leaves one behind.
        core.set_pill(Some(&first), "Created workspace-2".into(), true);
        let stale =
            (core.deps.clock.now() - chrono::Duration::seconds(PILL_SECONDS as i64)).to_rfc3339();
        for client in [&first, &second] {
            core.model
                .client_mut(client)
                .unwrap()
                .pill
                .as_mut()
                .unwrap()
                .at = stale.clone();
        }
        core.tick();
        assert!(core.model.client(&first).unwrap().pill.is_none());
        assert!(
            core.model.client(&second).unwrap().pill.is_none(),
            "the second client's pill goes too, or a stale line sits there for the session"
        );
    }

    /// A pill for a caller with no client draws nothing rather than picking a screen. Every
    /// job outcome goes through `set_pill`, and a `workspace.create` from the command line
    /// with nothing attached is the call that arrives here with `None`.
    #[test]
    fn a_pill_with_no_client_marks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let client = attached(&mut core);
        core.view_dirty = false;
        core.set_pill(None, "Created workspace-1".into(), true);
        assert!(core.model.client(&client).unwrap().pill.is_none());
        assert!(!core.view_dirty);
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

    /// `workspace.focus` reports a workspace with no tab rather than quietly making one.
    ///
    /// `ensure_every_workspace_has_a_tab` is the one owner of that invariant: `Core::new` runs
    /// it once and `apply_side_effects` runs it as its last statement, which is after every
    /// dispatch. So no handler can ever be handed a workspace without a tab, and a handler
    /// that made one would be a second owner whose repair no input could reach and no test
    /// could check.
    ///
    /// A unit test, and the state is set directly rather than staged, because there is no
    /// front door: every path that could leave a workspace tab-less runs the invariant on the
    /// way out. **The cost is that the scenario is asserted rather than exercised.** What it
    /// buys is a real verdict for the mutant on this branch, which was a permanent survivor
    /// while the branch repaired instead of reporting. The link this does not exercise is
    /// "the core really can hand a handler this state", and that link is exactly the one the
    /// invariant exists to make impossible.
    ///
    /// `add_slot` is the right way in: it is what registers a worktree that already exists,
    /// and it leaves `tabs` empty and `last_tab` `None` together, which is the same state the
    /// invariant finds and fills.
    #[test]
    fn focusing_a_workspace_with_no_tab_reports_the_broken_invariant() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let client = attached(&mut core);
        let project = core
            .model
            .projects
            .first()
            .map(|p| p.id.clone())
            .expect("the project root is registered as a project");
        let (ws, _) = core
            .model
            .add_slot(&project, 1, dir.path().join("workspace-1"))
            .unwrap();
        assert!(core.model.workspace(&ws).unwrap().tabs.is_empty());
        let was_in = core.model.client(&client).unwrap().workspace.clone();
        let last = core.model.last_workspace.clone();

        let method = Method::from_request(
            "workspace.focus",
            serde_json::json!({ "workspace": ws.as_str() }),
        )
        .expect("workspace.focus takes these params");
        let err = core
            .dispatch(method, Some(client.clone()))
            .expect_err("a workspace with no tab has nothing to show");

        assert_eq!(err.code, ErrorCode::Internal, "{err}");
        assert!(err.message.contains(ws.as_str()), "{err}");
        assert!(
            err.message.contains("ensure_every_workspace_has_a_tab"),
            "the message names what should have run, so a reader can act on it: {err}"
        );
        // It refused rather than half working. The dispatch that just failed still ran
        // `apply_side_effects` on its way out, so the workspace has a tab by now; what must
        // not have moved is the client and the model's last workspace.
        assert_eq!(core.model.client(&client).unwrap().workspace, was_in);
        assert_eq!(core.model.last_workspace, last);
    }

    /// The register `only_the_expected_methods_still_answer_unavailable` and
    /// `every_unavailable_arm_in_dispatch_is_listed_in_still_unbuilt` both check against:
    /// every method declared in `domux_core::api` that no task has given a handler yet, with
    /// the params to call it by.
    ///
    /// M2 emptied it, which was the end state it was built to reach, and M3 filled it again:
    /// Task 5 declares thirteen methods in one commit so that every caller reads one shape of
    /// the API from the start of the milestone, and Tasks 10 to 18 fill them in one at a
    /// time. **When M3 finishes this holds exactly `agent.send`, `agent.read` and
    /// `agent.wait`** - the three verbs M4 fills - **and nothing else.** Anything else still
    /// on it is a method that would reach the cut-over answering "not built" to a reader who
    /// has no way to know that from the outside, which is the failure this register exists to
    /// prevent.
    ///
    /// `workspace.resume` was never here. M2 gave it a handler that refused in its own words,
    /// "resume arrives with agents in M3", and M3 replaced that with the real resume. Both
    /// directions key off the words "is not built yet", so a method that refuses in words of its
    /// own has to stay off: direction A asserts that message on everything listed here, and
    /// direction B only scans arms in `dispatch` that carry it.
    ///
    /// Task 18 built `agent.resume`, the last of the ten M3 fills, so what is left is exactly
    /// the three verbs M4 fills.
    const STILL_UNBUILT: &[(&str, &str)] = &[
        ("agent.send", r#"{"text": "hello"}"#),
        ("agent.read", "{}"),
        ("agent.wait", "{}"),
    ];

    /// Direction A of the register: implementing one of `STILL_UNBUILT` must fail this test
    /// until the implementer removes that method's line here and from the stub block in
    /// `api::dispatch`, so removal is forced rather than remembered.
    #[test]
    fn only_the_expected_methods_still_answer_unavailable() {
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
    /// `Method::NAMES` to see which answer `unavailable`: most of the others have side
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
        let mut scan_reached_a_known_arm = false;
        for (pattern, arm_body) in split_match_arms(body) {
            // `ServerInfo` is dispatch's first arm and the one method that cannot be stubbed,
            // so finding it proves the source scan parsed the function.
            scan_reached_a_known_arm |= pattern.contains("ServerInfo");
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
        // This guard used to be `!from_dispatch.is_empty()`, on the assumption that some method
        // is always unbuilt. That assumption expired: `STILL_UNBUILT` reaching empty is the end
        // state this register was built to reach, and at that point a working scan and a broken
        // one both find nothing. So the self-check has to prove the **scan** ran rather than
        // that it found something, which is what it was always for.
        assert!(
            scan_reached_a_known_arm,
            "the scan did not find dispatch's ServerInfo arm, so it is broken rather than \
             correctly finding no unbuilt method"
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

    /// A project with one slot and a client in it, for the destructive handlers.
    ///
    /// `add_slot` is the right way in: it is what registers a worktree that already exists,
    /// and the path is inside the test's own temporary directory, which is the only place
    /// anything in this file is ever allowed to point a delete at.
    fn a_slot(core: &mut Core, dir: &Path) -> WorkspaceId {
        a_slot_at(core, dir.join("workspace-1"))
    }

    fn a_slot_at(core: &mut Core, path: PathBuf) -> WorkspaceId {
        let project = core
            .model
            .projects
            .first()
            .map(|p| p.id.clone())
            .expect("the project root is registered as a project");
        let (ws, _) = core
            .model
            .add_slot(&project, 1, path)
            .expect("a fresh model has the number free");
        ws
    }

    /// The outcome of the one job this core started, or a failure naming the wait.
    async fn one_job(rx: &mut mpsc::Receiver<CoreMsg>) -> JobOutcome {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                match rx.recv().await {
                    Some(CoreMsg::JobFinished { outcome, .. }) => return outcome,
                    Some(_) => continue,
                    None => panic!("the core's channel closed before the job reported"),
                }
            }
        })
        .await
        .expect("the job did not report")
    }

    fn delete_question(core: &mut Core) -> String {
        let method = Method::from_request(
            "workspace.delete",
            serde_json::json!({ "workspace": "workspace-1" }),
        )
        .expect("workspace.delete takes these params");
        core.dispatch(method, None)
            .expect_err("a delete with no consent is a question")
            .message
    }

    /// The question names the branch the branch provider observed, not the one the handle is
    /// named after.
    ///
    /// The fact is set directly rather than fetched. A harness cannot reach this: its clock
    /// is fixed, so a branch fact that has arrived once is never due again and cannot be made
    /// to change, and racing a checkout against the provider's first tick is a test that
    /// inverts its own verdict when it flakes. The cost is that the scenario is asserted
    /// rather than exercised; what the fact is read for is one line in the handler, and the
    /// job that acts on the branch reads git instead (`branch_now`).
    ///
    /// The slot is put on `feat/auth-cleanup` because a slot whose branch equals its handle
    /// cannot tell "the fact" from "the handle": both answer `workspace-1`.
    #[test]
    fn the_delete_question_names_the_branch_that_was_observed() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let _client = attached(&mut core);
        let ws = a_slot(&mut core, dir.path());
        core.facts.set(
            FactKey::workspace(&ws, domux_core::facts::FACT_BRANCH),
            Some(a_fact("feat/auth-cleanup", None)),
        );
        let question = delete_question(&mut core);
        assert!(
            question.contains("and the local branch feat/auth-cleanup and"),
            "{question}"
        );
    }

    /// And with no fact it names none. A question that said "the local branch workspace-1"
    /// would promise to delete a branch nothing had looked at (principle 4).
    ///
    /// The pair is the point: with only the test above, an implementation that always printed
    /// the fact and one that fell back to the handle are indistinguishable.
    #[test]
    fn the_delete_question_names_no_branch_when_none_was_observed() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let _client = attached(&mut core);
        let _ws = a_slot(&mut core, dir.path());
        let question = delete_question(&mut core);
        assert!(
            question.contains("and its local branch and"),
            "the branch is spoken of without being named: {question}"
        );
        assert!(
            !question.contains("the local branch workspace-1"),
            "and the handle is not offered as one: {question}"
        );
    }

    /// The handler carries the branch fact into the job, so the reconciliation has something
    /// to reconcile.
    ///
    /// This is the caller's half, and it has to be tested through the caller: the refusal
    /// lives in the job, and a test that hands the job an expectation directly proves the half
    /// nothing questioned. With `expected_branch: None` in the handler the job would go ahead
    /// and delete `workspace-1` while the reader had been told `feat/auth-cleanup`, and every
    /// test that stops at the job stays green.
    ///
    /// The fact is set on the registry directly, for the reason the sibling test gives: a fixed
    /// clock never makes a branch fact due again, so one cannot be made to go stale through the
    /// harness. What is exercised here is the wiring from `ctx.facts` to the job, which is the
    /// line the mutant lives on.
    #[tokio::test]
    async fn the_handler_carries_the_branch_the_question_named_into_the_job() {
        let (_tmp, repo) = crate::testing::repo_with_origin("main");
        let path = crate::git::slot_path(&repo, 1);
        crate::git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (mut core, mut rx) = core_with_providers(dir.path(), Vec::new());
        let ws = a_slot_at(&mut core, path.clone());
        // What the branch provider last saw. The worktree is really on `workspace-1`, so this
        // is a fact that has gone stale, which is the state the reconciliation is about.
        core.facts.set(
            FactKey::workspace(&ws, domux_core::facts::FACT_BRANCH),
            Some(a_fact("feat/auth-cleanup", None)),
        );

        core.dispatch(
            Method::from_request(
                "workspace.delete",
                serde_json::json!({"workspace": "workspace-1", "yes": true}),
            )
            .expect("params"),
            None,
        )
        .expect("the handler queues the job and defers its answer");

        match one_job(&mut rx).await {
            JobOutcome::Failed { message, code } => {
                assert_eq!(code, ErrorCode::Conflict, "{message}");
                assert!(message.contains("not feat/auth-cleanup"), "{message}");
            }
            _ => panic!("the delete went ahead on a branch the question did not name"),
        }
        assert!(path.is_dir(), "and the worktree is still there");
    }

    /// The job refuses when the worktree has left the branch the question named, and goes
    /// ahead when it has not.
    ///
    /// Called directly rather than through the API, because the disagreement cannot be staged
    /// end to end: the harness clock is fixed, so a branch fact that has arrived once is never
    /// due again and cannot be made to go stale, and racing a checkout against the provider's
    /// first tick is a test that inverts its own verdict when it flakes. The state the race
    /// would produce is set here instead and the question asked exactly. The cost is that the
    /// scenario is asserted rather than exercised.
    ///
    /// Both directions, because a job that refused every time and a job that refused the right
    /// time answer the same on the first case alone. And the refusal asserts the worktree is
    /// still on disk: an error code does not tell a delete that refused from one that removed
    /// and then complained.
    #[test]
    fn a_delete_refuses_when_the_worktree_has_left_the_branch_the_question_named() {
        let (_tmp, repo) = crate::testing::repo_with_origin("main");
        let path = crate::git::slot_path(&repo, 1);
        crate::git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
        let ws = WorkspaceId("w_1".into());
        let delete = |expected: &str| {
            delete_workspace(
                ws.clone(),
                "workspace-1".to_string(),
                &repo,
                &path,
                Some(expected.to_string()),
                None,
                false,
            )
        };

        match delete("feat/auth-cleanup") {
            JobOutcome::Failed { message, code } => {
                assert_eq!(code, ErrorCode::Conflict, "{message}");
                assert!(message.contains("is on workspace-1 now"), "{message}");
                assert!(message.contains("not feat/auth-cleanup"), "{message}");
            }
            _ => panic!("a delete of a branch nobody consented to must not go ahead"),
        }
        assert!(path.is_dir(), "and it refused before it removed anything");

        assert!(
            matches!(delete("workspace-1"), JobOutcome::Deleted { .. }),
            "and the branch the question named is the one it removes"
        );
        assert!(!path.exists());
    }

    /// `esc` on the delete question starts no job, and `y` starts one.
    ///
    /// The observable is the job, not the disk. A delete from a key queues work on a blocking
    /// task and answers nothing, so a harness test that closes the question and then looks at
    /// the worktree is racing the job it means to say never started - and it wins that race
    /// either way. Measured: with `input::confirmed` forced to `true`, so `esc` acts as `y`,
    /// `a_key_other_than_y_closes_the_delete_question` stays green.
    ///
    /// `JobFinished` is the message the job sends however it ends, so waiting for one is
    /// waiting for the mechanism itself. The two halves share one window, which is what makes
    /// the negative worth anything: if two seconds were not enough for a job to report, the
    /// `y` half fails rather than the `esc` half passing quietly.
    ///
    /// The job runs against a slot path inside this test's own temporary directory that no
    /// repository was ever built at, so `git rev-parse` refuses immediately and nothing is
    /// removed anywhere.
    #[tokio::test]
    async fn esc_on_the_delete_question_starts_no_job_and_y_starts_one() {
        for (key, expected) in [('y', true), ('n', false)] {
            let dir = tempfile::tempdir().unwrap();
            let (mut core, mut rx) = core_with_providers(dir.path(), Vec::new());
            let client = attached(&mut core);
            let _ws = a_slot(&mut core, dir.path());
            core.run_action(
                &client,
                &domux_core::keymap::Action {
                    method: "workspace.delete".into(),
                    args: vec!["workspace-1".into()],
                },
            );
            assert!(
                matches!(
                    core.model.client(&client).unwrap().overlay,
                    Some(Overlay::Confirm(ConfirmKind::DeleteWorkspace(_)))
                ),
                "the question is open before the answer"
            );
            crate::input::route_key(
                &mut core,
                &client,
                domux_term::KeyEvent {
                    key: domux_term::Key::Char(key),
                    mods: domux_term::Mods::empty(),
                    action: domux_term::KeyAction::Press,
                },
            );
            let started = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match rx.recv().await {
                        Some(CoreMsg::JobFinished { .. }) => return true,
                        Some(_) => continue,
                        None => return false,
                    }
                }
            })
            .await
            .unwrap_or(false);
            assert_eq!(started, expected, "{key} started a job: {started}");
        }
    }

    /// A key carrying a client the model does not hold opens no question, anywhere.
    ///
    /// The branch is reachable: `Core::run_action` never checks that the id it is handed is
    /// attached, so `from_key` does not close the path. That makes it a refusal to test rather
    /// than an argument to write down, which is Task 19's ruling on the same shape - a branch
    /// no run can reach is a mutant that can never die, and a survivor that can never die
    /// teaches the next reader that survivors are normal.
    ///
    /// The attached client is what makes the second assertion able to fail: with nobody else
    /// attached, "no client was given the question" and "there was no client" are the same
    /// sentence. It is the view a wrong answer would land on - a handler that resolved the
    /// dropped id to some other client would push the question here.
    ///
    /// Not `Ctx::view`'s fallback, which an earlier version of this comment named:
    /// `run_action` always passes `Some(client)`, so the `or_else` never evaluates on this
    /// path. The mutant this kills is `ask` looking the client up with `most_recent_client`.
    #[test]
    fn a_key_from_a_client_that_is_not_attached_opens_no_question_anywhere() {
        for method in ["workspace.delete", "workspace.clear"] {
            let dir = tempfile::tempdir().unwrap();
            let mut core = core(dir.path());
            let watching = attached(&mut core);
            let _ws = a_slot(&mut core, dir.path());
            let gone = ClientId("c_gone".into());

            let err = core
                .dispatch_from_key(
                    Method::from_request(method, serde_json::json!({"workspace": "workspace-1"}))
                        .expect("params"),
                    Some(gone.clone()),
                )
                .expect_err("there is no screen to put the question on");

            assert_eq!(err.code, ErrorCode::NotFound, "{method}: {err}");
            assert!(err.message.contains(gone.as_str()), "{method}: {err}");
            assert!(
                core.model.client(&watching).unwrap().overlay.is_none(),
                "{method}: the question must not land on somebody else's screen"
            );
        }
    }

    /// A confirmation opened from inside the switcher goes back to the switcher, on `esc` and
    /// on `y` alike, rather than leaving it stranded under a closed overlay.
    ///
    /// The switcher is set directly because no input reaches this yet: inside the switcher
    /// only the key bound to `focus.pane` is read until Task 14 lands `[keys.list]`, and a
    /// leader chord cannot start while any overlay is open. The mechanism is the real one -
    /// `api::workspace::ask` pushes and `input::pop_confirmation` pops - and this is the only
    /// way to ask the question at all. The cost is that the scenario is asserted rather than
    /// exercised.
    ///
    /// Both keys, because they leave by different lines: `esc` pops and stops, `y` pops and
    /// then dispatches, and a pop written into one of the two is a switcher that survives
    /// being declined and vanishes on being accepted.
    #[tokio::test]
    async fn a_confirmation_over_the_switcher_goes_back_to_the_switcher() {
        for key in ['y', 'n'] {
            let dir = tempfile::tempdir().unwrap();
            let mut core = core(dir.path());
            let client = attached(&mut core);
            let ws = a_slot(&mut core, dir.path());
            core.model
                .client_mut(&client)
                .expect("attached")
                .push_overlay(Overlay::Switcher);

            core.run_action(
                &client,
                &domux_core::keymap::Action {
                    method: "workspace.delete".into(),
                    args: vec!["workspace-1".into()],
                },
            );
            assert_eq!(
                core.model.client(&client).unwrap().overlay,
                Some(Overlay::Confirm(ConfirmKind::DeleteWorkspace(ws.clone()))),
                "the question is on top"
            );
            assert_eq!(
                core.model.client(&client).unwrap().overlay_under,
                Some(Overlay::Switcher),
                "and the switcher is under it, not gone"
            );

            crate::input::route_key(
                &mut core,
                &client,
                domux_term::KeyEvent {
                    key: domux_term::Key::Char(key),
                    mods: domux_term::Mods::empty(),
                    action: domux_term::KeyAction::Press,
                },
            );
            let view = core.model.client(&client).unwrap();
            assert_eq!(
                view.overlay,
                Some(Overlay::Switcher),
                "{key} puts the reader back where the question was asked"
            );
            assert_eq!(view.overlay_under, None, "{key} left nothing stranded");
            // `Switcher`, not the generic `Overlay`. This asserted `Overlay` when it was
            // written, which is what `close_overlay` produced before Task 20 gave
            // `ClientView::focus_after_pop` the job of naming the switcher as the box it is.
            // The weaker value was what the code did; this one is what the test is called.
            assert_eq!(
                view.focus,
                Focus::Region(RegionKind::Switcher),
                "{key} left the keys in the box that is still open"
            );
        }
    }

    /// Fetched at the instant every test core's clock reads, so it is inside its time to
    /// live for as long as the test runs.
    fn a_fact(text: &str, state: Option<FactState>) -> Fact {
        stamped(text, state, "2026-09-04T14:32:00")
    }

    fn stamped(text: &str, state: Option<FactState>, at: &str) -> Fact {
        Fact::new(
            text,
            state,
            FixedClock::at(at).0.to_rfc3339(),
            Duration::from_secs(600),
        )
    }

    /// The next fact answer on the core's own channel. A bound on failure, not a wait: it
    /// returns the moment the message lands.
    async fn next_answer(rx: &mut mpsc::Receiver<CoreMsg>) -> (FactKey, Option<Fact>) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let CoreMsg::FactFetched { key, fact } =
                    rx.recv().await.expect("the core channel stays open")
                {
                    return (key, fact);
                }
            }
        })
        .await
        .expect("the answer comes back")
    }

    /// The first workspace of the core's implicit project.
    fn first_workspace(core: &Core) -> WorkspaceId {
        core.model.first_workspace().expect("one workspace")
    }

    /// A model with one project at `root`, holding `main` and one slot at `slot_path`, saved
    /// as a state file at `<dir>/state.json`. The path back is what a caller varies.
    fn state_file_with_a_slot_at(dir: &Path, root: &Path, slot_path: &Path) -> (PathBuf, Model) {
        let mut model = Model::new(7);
        let (project, _main, _) = model.add_folder_project(root.to_path_buf()).unwrap();
        model
            .project_mut(&project)
            .expect("the project is there")
            .workspaces
            .push(domux_core::model::Workspace {
                id: WorkspaceId("w_5101".into()),
                handle: domux_core::model::WorkspaceHandle::Slot(1),
                name: None,
                path: slot_path.to_path_buf(),
                tabs: Vec::new(),
                last_tab: None,
            });
        let saved = dir.join("state.json");
        std::fs::write(
            &saved,
            serde_json::to_string(&state_file::snapshot(&model, "2026-09-04T14:32:00+01:00"))
                .unwrap(),
        )
        .unwrap();
        (saved, model)
    }

    /// A record is pruned only when the filesystem says there is nothing there, never when it
    /// says it cannot tell.
    ///
    /// A directory with no permissions is the reachable version of this. An unplugged disk and
    /// a network mount that timed out reach `prune_missing_paths` the same way and cannot be
    /// built in a test; a `chmod 000` on a parent gives `PermissionDenied` for the path under
    /// it, which is the same `Err` kind question the code asks. `Path::is_dir`, which this
    /// replaced, answers false for all four.
    ///
    /// The mode goes back before the assertions so the temp directory can still be removed
    /// when one of them fails.
    #[test]
    fn a_record_whose_path_cannot_be_read_is_kept_rather_than_pruned() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        let root = locked.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let (saved, _) = state_file_with_a_slot_at(dir.path(), &root, &root.join("slot"));
        std::fs::create_dir_all(root.join("slot")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let core = core_with(dir.path(), Vec::new(), &saved).0;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            core.notes.is_empty(),
            "nothing was said, because nothing was known: {:?}",
            core.notes
        );
        assert_eq!(
            core.model
                .projects
                .iter()
                .find(|p| p.root == root)
                .map(|p| p.workspaces.len()),
            Some(2),
            "the project and both its workspaces are still there"
        );
        assert!(
            !crate::persist::with_suffix(&saved, ".pruned").exists(),
            "and no backup was made, because nothing was about to be deleted"
        );
    }

    /// A prune keeps the state as it was under a name nothing rotates, so the records it took
    /// are recoverable by hand.
    ///
    /// `.bak` is not that name: `write_atomic` moves the current file into it on every write,
    /// and the prune's own events make the very next batch structural, so `.bak` holds the
    /// pre-prune model for one write and the attaching client's `PaneResized` takes the second.
    #[test]
    fn a_prune_keeps_the_state_it_started_from_and_a_clean_start_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj-a");
        std::fs::create_dir_all(&root).unwrap();
        let (saved, _) = state_file_with_a_slot_at(dir.path(), &root, &root.join("gone"));
        let before = std::fs::read_to_string(&saved).unwrap();
        let kept = crate::persist::with_suffix(&saved, ".pruned");

        let core = core_with(dir.path(), Vec::new(), &saved).0;
        assert_eq!(core.notes.len(), 1, "the fixture did prune something");
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap(),
            before,
            "the file kept aside is the one the server read, before anything was taken from it"
        );
        assert!(
            before.contains("w_5101"),
            "which is the point: the record that went is still in it"
        );
        assert_eq!(
            std::fs::read_to_string(&saved).unwrap(),
            before,
            "and it is a copy: the file the server goes on writing is still where it was"
        );

        // A second start over the corrected state has nothing to prune, so it must not rotate
        // the backup away. Same directory, so the file it would overwrite is the one above.
        std::fs::write(
            &saved,
            serde_json::to_string(&state_file::snapshot(
                &core.model,
                "2026-09-04T14:32:00+01:00",
            ))
            .unwrap(),
        )
        .unwrap();
        let again = core_with(dir.path(), Vec::new(), &saved).0;
        assert!(again.notes.is_empty(), "nothing left to prune");
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap(),
            before,
            "so the copy from the start that did prune is still the one on disk"
        );
    }

    /// A project going takes the same copy aside as a workspace going.
    ///
    /// Its own test because the two branches make the copy separately: the project branch
    /// makes it and sets the flag, and the workspace branch makes it only if the project
    /// branch did not. A fixture where both kinds go at once exercises the first branch and
    /// skips the second, so it can never tell whether the second one was there at all - and a
    /// fixture where only a workspace goes cannot see the first.
    #[test]
    fn a_project_that_is_pruned_also_keeps_the_state_it_started_from() {
        let dir = tempfile::tempdir().unwrap();
        // Never created, so it is the project's root that is gone rather than a slot's path.
        let root = dir.path().join("never-made");
        let (saved, _) = state_file_with_a_slot_at(dir.path(), &root, &root.join("slot"));
        let before = std::fs::read_to_string(&saved).unwrap();
        let kept = crate::persist::with_suffix(&saved, ".pruned");

        let core = core_with(dir.path(), Vec::new(), &saved).0;
        assert_eq!(
            core.notes,
            vec!["Removed never-made: its folder is gone".to_string()],
            "the project is what went"
        );
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap(),
            before,
            "and the state as it was is kept, the same as when a slot goes"
        );
    }

    /// A path that is there but is not a directory is not missing. Something is at that name,
    /// and deleting the record of it is the outcome the `NotFound` check exists to avoid.
    ///
    /// The old probe was `!path.is_dir()`, which answers "prune it" here.
    #[test]
    fn a_path_that_is_there_but_is_not_a_directory_is_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj-c");
        std::fs::create_dir_all(&root).unwrap();
        let slot = root.join("slot");
        std::fs::write(&slot, "not a directory\n").unwrap();
        let (saved, _) = state_file_with_a_slot_at(dir.path(), &root, &slot);

        let core = core_with(dir.path(), Vec::new(), &saved).0;
        assert!(
            core.notes.is_empty(),
            "nothing was pruned: {:?}",
            core.notes
        );
        assert!(
            core.model
                .workspace(&WorkspaceId("w_5101".into()))
                .is_some(),
            "the record for the name that is taken is still there"
        );
    }

    /// The start forgets a pruned workspace's facts, before any tick has had the chance to.
    ///
    /// A unit test because `start_due_fetches` calls `forget_deleted` on every tick, so in the
    /// harness the tick is a second cause standing in for the call in `Core::new`: the
    /// interface test named for this passes with the two in either order. Here no tick has run,
    /// so the only thing that can have dropped the fact is the start.
    ///
    /// The neighbour's fact is asserted too. Without it a start that forgot every fact it
    /// loaded would pass.
    #[test]
    fn the_start_forgets_a_pruned_workspaces_facts_before_any_tick_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj-b");
        std::fs::create_dir_all(&root).unwrap();
        let (saved, model) = state_file_with_a_slot_at(dir.path(), &root, &root.join("gone"));
        let kept_workspace = model.first_workspace().expect("main");
        let pruned = WorkspaceId("w_5101".into());
        let mut cache = FactRegistry::new();
        for id in [&kept_workspace, &pruned] {
            cache.set(
                FactKey::workspace(id, domux_core::facts::FACT_PR),
                Some(Fact::new(
                    "PR#212",
                    Some(FactState::Open),
                    FixedClock::at("2026-09-04T14:32:00").0.to_rfc3339(),
                    Duration::from_secs(600),
                )),
            );
        }
        cache.save_cache(
            &crate::facts::pr_cache_path(&dir.path().join("state")),
            &[domux_core::facts::FACT_PR],
        );

        let core = core_with(dir.path(), Vec::new(), &saved).0;
        assert!(
            core.notes.len() == 1 && core.model.workspace(&pruned).is_none(),
            "the fixture pruned the workspace the fact is about: {:?}",
            core.notes
        );
        assert!(
            core.facts
                .get(&FactKey::workspace(&pruned, domux_core::facts::FACT_PR))
                .is_none(),
            "its pull request went with it, at the start rather than at the first tick"
        );
        assert!(
            core.facts
                .get(&FactKey::workspace(
                    &kept_workspace,
                    domux_core::facts::FACT_PR
                ))
                .is_some(),
            "and the workspace that stayed kept what the cache knew about it"
        );
    }

    /// A note names a workspace the way the Projects box does: its name when it has one, and
    /// its handle otherwise.
    ///
    /// A state file, and not the API, because `workspace.rename` is Task 19's and nothing in
    /// this milestone can name a workspace through the server yet. Without a name in the
    /// fixture the handle and the display name are the same string, and a note built from
    /// either reads correctly.
    #[test]
    fn a_note_names_a_pruned_workspace_by_its_name_when_it_has_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let mut model = Model::new(7);
        let (project, _main, _) = model.add_folder_project(root.clone()).unwrap();
        let slot = WorkspaceId("w_5101".into());
        model
            .project_mut(&project)
            .expect("the project is there")
            .workspaces
            .push(domux_core::model::Workspace {
                id: slot.clone(),
                handle: domux_core::model::WorkspaceHandle::Slot(1),
                name: None,
                // Under the project's root, which is there, so only this path is gone.
                path: root.join(".domux/worktrees/workspace-1"),
                tabs: Vec::new(),
                last_tab: None,
            });
        model
            .rename_workspace(&slot, Some("auth cleanup".to_string()))
            .unwrap();
        let saved = dir.path().join("state.json");
        std::fs::write(
            &saved,
            serde_json::to_string(&state_file::snapshot(&model, "2026-09-04T14:32:00+01:00"))
                .unwrap(),
        )
        .unwrap();

        let core = core_with(dir.path(), Vec::new(), &saved).0;
        assert_eq!(
            core.notes,
            vec!["Pruned auth cleanup: its worktree is gone".to_string()],
            "the reader is told about the workspace by the name they gave it"
        );
        assert_eq!(
            core.model.projects.len(),
            1,
            "and the project, whose root is there, stayed"
        );
    }

    /// The prune records what it took away, so the first batch writes the corrected model.
    ///
    /// Nothing can subscribe before `Core::new` returns, so these events reach no reader: what
    /// they do is make the first batch structural, and a structural batch persists. Without
    /// them a server killed rather than stopped would keep a state file naming a directory
    /// that is not there, and would say the same thing again at the next start.
    ///
    /// A unit test rather than a harness one, and deliberately: attaching a client resizes
    /// the panes to fit inside their boxes, `sync_pane_sizes` publishes `PaneResized`, and
    /// that is structural too - so through the harness the state file is written either way
    /// and the events under test would have a second cause standing in for them.
    ///
    /// `pruned` is asserted rather than only the variant. It is the field that tells a reader
    /// of the event stream a deletion from a path that went away underneath, and
    /// `remove_workspace` produces the identical event with it false.
    #[test]
    fn the_prune_records_what_it_took_away_as_events() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let mut model = Model::new(7);
        let (kept, _main, _) = model.add_folder_project(root.clone()).unwrap();
        let slot = WorkspaceId("w_5101".into());
        model
            .project_mut(&kept)
            .expect("the project is there")
            .workspaces
            .push(domux_core::model::Workspace {
                id: slot.clone(),
                handle: domux_core::model::WorkspaceHandle::Slot(1),
                name: None,
                path: root.join(".domux/worktrees/workspace-1"),
                tabs: Vec::new(),
                last_tab: None,
            });
        // A second project, whose root was never made, so both loops have something to say.
        let (ghost, _, _) = model
            .add_folder_project(dir.path().join("ghost"))
            .expect("a second project");
        let saved = dir.path().join("state.json");
        std::fs::write(
            &saved,
            serde_json::to_string(&state_file::snapshot(&model, "2026-09-04T14:32:00+01:00"))
                .unwrap(),
        )
        .unwrap();

        let core = core_with(dir.path(), Vec::new(), &saved).0;
        assert!(
            core.pending_events.contains(&Event::ProjectRemoved {
                project: ghost.clone(),
                name: "ghost".into(),
            }),
            "the project that went is in the batch: {:?}",
            core.pending_events
        );
        assert!(
            core.pending_events.contains(&Event::WorkspaceDeleted {
                project: kept.clone(),
                workspace: slot.clone(),
                handle: "workspace-1".into(),
                pruned: true,
            }),
            "and so is the slot, marked as a prune rather than as a deletion: {:?}",
            core.pending_events
        );
    }

    /// A key release is not a key press, so it does not read a note on the reader's behalf.
    ///
    /// Both halves of one press reach `Core::key` when the client's terminal reports releases,
    /// and clearing on the release would take the note away one event before `route_key` had
    /// decided what the press meant. The harness sends only presses, so this is the only place
    /// the two can be told apart.
    ///
    /// The switcher is opened through its own handler rather than by setting the overlay
    /// field. `api::switcher::open` sets the overlay and the region together, and a fixture
    /// that set only one of them would be a state no running server can be in.
    #[test]
    fn a_key_release_in_a_box_leaves_a_note_where_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let client = attached(&mut core);
        core.notes = vec!["Pruned workspace-2: its worktree is gone".into()];
        core.dispatch(
            Method::SwitcherOpen(domux_core::api::ClientParams {
                client: Some(client.clone()),
            }),
            Some(client.clone()),
        )
        .expect("switcher.open");
        let key = |action| domux_term::KeyEvent {
            key: domux_term::Key::Char('j'),
            mods: domux_term::Mods::empty(),
            action,
        };

        core.key(&client, key(domux_term::KeyAction::Release));
        assert_eq!(
            core.notes.len(),
            1,
            "the note is still there for the press to clear"
        );
        core.key(&client, key(domux_term::KeyAction::Press));
        assert!(core.notes.is_empty(), "and the press clears it");
    }

    fn fact_events(core: &Core) -> Vec<Event> {
        core.pending_events
            .iter()
            .filter(|e| matches!(e, Event::FactUpdated { .. }))
            .cloned()
            .collect()
    }

    fn pr_cache(core: &Core) -> String {
        std::fs::read_to_string(crate::facts::pr_cache_path(&core.state_dir))
            .unwrap_or_else(|_| String::new())
    }

    #[test]
    fn a_fact_that_arrives_reaches_the_registry_the_cache_and_the_event_stream() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let key = FactKey::workspace(&first_workspace(&core), domux_core::facts::FACT_PR);
        core.pending_events.clear();
        core.view_dirty = false;
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        assert_eq!(
            core.facts.get(&key).map(|f| f.text.as_str()),
            Some("PR#212")
        );
        assert_eq!(
            fact_events(&core),
            vec![Event::FactUpdated {
                key: key.clone(),
                present: true
            }]
        );
        assert!(core.view_dirty, "the switcher shows the number");
        assert!(
            pr_cache(&core).contains("PR#212"),
            "the pull request survives a restart: {}",
            pr_cache(&core)
        );
    }

    #[test]
    fn an_answer_that_repeats_what_the_screen_shows_announces_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let key = FactKey::workspace(&first_workspace(&core), domux_core::facts::FACT_PR);
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        core.pending_events.clear();
        core.view_dirty = false;
        // The same answer, looked up a minute later.
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(stamped(
                "PR#212",
                Some(FactState::Open),
                "2026-09-04T14:33:00",
            )),
        });
        assert_eq!(
            fact_events(&core),
            Vec::new(),
            "a provider that looks again every few seconds must not redraw the screen every few seconds"
        );
        assert!(!core.view_dirty);
    }

    /// The file's own stamp is what the sweep on the next start reads. It has to move with
    /// the registry's, and the thing that decides whether to draw is no help here: `shown`
    /// ignores the stamp on purpose, so an answer that repeats yesterday's number is "no
    /// change" to the screen and a whole new age to the cache.
    #[test]
    fn a_pull_request_that_did_not_change_still_refreshes_the_stamp_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let key = FactKey::workspace(&first_workspace(&core), domux_core::facts::FACT_PR);
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(stamped(
                "PR#212",
                Some(FactState::Open),
                "2026-09-04T14:32:00",
            )),
        });
        assert!(pr_cache(&core).contains("14:32:00"), "{}", pr_cache(&core));
        // The same number, looked up ten minutes later, which is past the time to live the
        // first write went to disk with.
        core.handle(CoreMsg::FactFetched {
            key,
            fact: Some(stamped(
                "PR#212",
                Some(FactState::Open),
                "2026-09-04T14:42:00",
            )),
        });
        let cache = pr_cache(&core);
        assert!(
            cache.contains("14:42:00") && !cache.contains("14:32:00"),
            "the cached number ages out and the switcher opens blank unless the file keeps \
             the stamp the registry has: {cache}"
        );
    }

    #[test]
    fn a_pull_request_that_changed_state_announces_itself_with_the_same_number() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let key = FactKey::workspace(&first_workspace(&core), domux_core::facts::FACT_PR);
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        core.pending_events.clear();
        core.view_dirty = false;
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("PR#212", Some(FactState::Merged))),
        });
        assert_eq!(
            fact_events(&core),
            vec![Event::FactUpdated {
                key: key.clone(),
                present: true
            }],
            "the number is the same and the colour is not, so the screen has to change"
        );
        assert!(core.view_dirty);
    }

    #[test]
    fn a_fetch_that_failed_removes_the_fact_and_announces_its_absence() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let key = FactKey::workspace(&first_workspace(&core), domux_core::facts::FACT_PR);
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        core.pending_events.clear();
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: None,
        });
        assert_eq!(
            core.facts.get(&key),
            None,
            "a failed fetch leaves nothing behind, never the last number it saw"
        );
        assert_eq!(
            fact_events(&core),
            vec![Event::FactUpdated {
                key: key.clone(),
                present: false
            }]
        );
        assert!(
            !pr_cache(&core).contains("PR#212"),
            "and the cache does not keep it either: {}",
            pr_cache(&core)
        );
    }

    #[test]
    fn only_the_pull_request_reaches_the_cache_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let w = first_workspace(&core);
        core.handle(CoreMsg::FactFetched {
            key: FactKey::workspace(&w, domux_core::facts::FACT_PR),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        core.handle(CoreMsg::FactFetched {
            key: FactKey::workspace(&w, domux_core::facts::FACT_BRANCH),
            fact: Some(a_fact("feat/auth-cleanup", None)),
        });
        let cache = pr_cache(&core);
        assert!(cache.contains("PR#212"), "{cache}");
        assert!(
            !cache.contains("feat/auth-cleanup"),
            "the branch is cheap to look up again and is never cached: {cache}"
        );
    }

    #[test]
    fn the_core_starts_with_the_cached_pull_request_and_forgets_one_about_a_workspace_it_lost() {
        let dir = tempfile::tempdir().unwrap();
        // The same seed and the same first steps as `Core::new`, so this is the id the core
        // below draws for its own workspace.
        let mut model = Model::new(7);
        model.add_folder_project(dir.path().join("proj")).unwrap();
        let w = model.first_workspace().unwrap();
        let gone = WorkspaceId("w_ffff".into());
        assert_ne!(w, gone);
        let mut cache = FactRegistry::new();
        let fetched_at = FixedClock::at("2026-09-04T14:32:00").0.to_rfc3339();
        for id in [&w, &gone] {
            cache.set(
                FactKey::workspace(id, domux_core::facts::FACT_PR),
                Some(Fact::new(
                    "PR#212",
                    Some(FactState::Open),
                    fetched_at.clone(),
                    Duration::from_secs(600),
                )),
            );
        }
        cache.save_cache(
            &crate::facts::pr_cache_path(&dir.path().join("state")),
            &[domux_core::facts::FACT_PR],
        );
        let core = core(dir.path());
        assert_eq!(
            core.facts
                .get(&FactKey::workspace(&w, domux_core::facts::FACT_PR))
                .map(|f| f.text.as_str()),
            Some("PR#212"),
            "the switcher opens with the last known number instead of a blank line"
        );
        assert_eq!(
            core.facts.len(),
            1,
            "and without the pull request of a workspace this server no longer has, whose id another workspace may draw"
        );
    }

    #[test]
    fn a_tick_forgets_the_facts_of_a_workspace_that_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let w = first_workspace(&core);
        let project = core.model.projects[0].id.clone();
        core.handle(CoreMsg::FactFetched {
            key: FactKey::workspace(&w, domux_core::facts::FACT_PR),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        assert_eq!(core.facts.len(), 1);
        core.model.remove_project(&project).unwrap();
        core.tick();
        assert!(
            core.facts.is_empty(),
            "the workspace is gone, so what was observed about it is gone with it"
        );
    }

    /// A provider that fails.
    struct Failing;

    impl crate::facts::FactProvider for Failing {
        fn name(&self) -> &str {
            domux_core::facts::FACT_BRANCH
        }
        fn interval(&self) -> Duration {
            Duration::from_secs(5)
        }
        fn scope(&self) -> crate::facts::ProviderScope {
            crate::facts::ProviderScope::Workspace
        }
        fn fetch(&self, _t: &crate::facts::FactTarget) -> Result<Option<Fact>, String> {
            Err("git: not a worktree".into())
        }
    }

    #[tokio::test]
    async fn a_provider_that_failed_sends_back_an_absence_and_never_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, mut rx) = core_with_providers(dir.path(), vec![Arc::new(Failing)]);
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        core.model.add_git_project(repo, "main".into()).unwrap();
        core.tick();
        let fact = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let CoreMsg::FactFetched { fact, .. } =
                    rx.recv().await.expect("the core channel stays open")
                {
                    return fact;
                }
            }
        })
        .await
        .expect("the answer comes back");
        assert_eq!(
            fact, None,
            "a provider that failed reports nothing, never a branch it did not read"
        );
    }

    /// A provider that will not answer until the test lets it.
    struct Gated {
        gate: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl crate::facts::FactProvider for Gated {
        fn name(&self) -> &str {
            domux_core::facts::FACT_BRANCH
        }
        fn interval(&self) -> Duration {
            Duration::from_secs(5)
        }
        fn scope(&self) -> crate::facts::ProviderScope {
            crate::facts::ProviderScope::Workspace
        }
        fn fetch(&self, _t: &crate::facts::FactTarget) -> Result<Option<Fact>, String> {
            // Bounded, so a fetch that wrongly ran on the core task fails the assertions
            // below rather than hanging the suite. Five seconds is far longer than the test
            // needs to reach `send` on the next line but one.
            let _ = self
                .gate
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
            Ok(Some(a_fact("feat/x", None)))
        }
    }

    /// What a provider found comes back to the core as a message and nothing else.
    #[tokio::test]
    async fn a_providers_answer_comes_back_to_the_core_as_a_message() {
        let dir = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (mut core, mut rx) = core_with_providers(
            dir.path(),
            vec![Arc::new(Gated {
                gate: Mutex::new(gate_rx),
            })],
        );
        // The implicit project is a plain folder, which has no branch to look up.
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        core.model.add_git_project(repo, "main".into()).unwrap();
        core.tick();
        gate_tx.send(()).unwrap();
        let (key, fact) = next_answer(&mut rx).await;
        assert_eq!(key.name, domux_core::facts::FACT_BRANCH);
        assert_eq!(
            fact.map(|f| f.text),
            Some("feat/x".to_string()),
            "what the provider found, carried back to the core as a message"
        );
    }

    /// A provider that has entered `fetch` and not left it.
    struct Blocking {
        entered: Arc<AtomicBool>,
        finished: Arc<AtomicBool>,
        gate: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl crate::facts::FactProvider for Blocking {
        fn name(&self) -> &str {
            domux_core::facts::FACT_BRANCH
        }
        fn interval(&self) -> Duration {
            Duration::ZERO
        }
        fn scope(&self) -> crate::facts::ProviderScope {
            crate::facts::ProviderScope::Workspace
        }
        fn fetch(&self, _t: &crate::facts::FactTarget) -> Result<Option<Fact>, String> {
            self.entered.store(true, Ordering::SeqCst);
            // The test drops the sender rather than waiting this out, so the pass path never
            // reaches five seconds. The bound is there so a fetch that ran on the core task
            // fails the assertion below instead of hanging the suite.
            let _ = self
                .gate
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
            self.finished.store(true, Ordering::SeqCst);
            Ok(None)
        }
    }

    /// The milestone's central constraint: nothing that shells out runs inside the core
    /// task, because one slow `gh` there stops every frame in every pane. The ordering is
    /// what proves it. `tick` returned, and the provider it started has not come out of
    /// `fetch`; running the fetch on the core task cannot produce that state, because `tick`
    /// could only have returned after `fetch` did.
    #[tokio::test]
    async fn tick_returns_while_a_provider_is_still_inside_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let entered = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let (mut core, _rx) = core_with_providers(
            dir.path(),
            vec![Arc::new(Blocking {
                entered: entered.clone(),
                finished: finished.clone(),
                gate: Mutex::new(gate_rx),
            })],
        );
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        core.model.add_git_project(repo, "main".into()).unwrap();
        core.tick();
        // Wait for the condition rather than a duration: the fetch has started somewhere.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !entered.load(Ordering::SeqCst) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(
            entered.load(Ordering::SeqCst),
            "the provider was started at all"
        );
        assert!(
            !finished.load(Ordering::SeqCst),
            "tick returned while the provider was still inside fetch, so fetch did not run on the core task"
        );
        // Let the blocking thread go, so the runtime can shut down at once.
        drop(gate_tx);
    }

    /// A provider that panics.
    struct Panicky {
        calls: Arc<AtomicUsize>,
    }

    impl crate::facts::FactProvider for Panicky {
        fn name(&self) -> &str {
            domux_core::facts::FACT_BRANCH
        }
        fn interval(&self) -> Duration {
            Duration::ZERO
        }
        fn scope(&self) -> crate::facts::ProviderScope {
            crate::facts::ProviderScope::Workspace
        }
        fn fetch(&self, _t: &crate::facts::FactTarget) -> Result<Option<Fact>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("this provider panics on purpose");
        }
    }

    /// Task 9 parses what `gh` printed, so a panic inside a fetch is not hypothetical. The
    /// answer that never arrives is the one failure that leaves a value on the screen with
    /// nothing left to replace it, so every fetch reports one way or the other.
    ///
    /// The panic message this prints on stderr is this test's own.
    #[tokio::test]
    async fn a_provider_that_panics_leaves_the_fact_absent_and_is_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let (mut core, mut rx) = core_with_providers(
            dir.path(),
            vec![Arc::new(Panicky {
                calls: calls.clone(),
            })],
        );
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (_, w, _) = core.model.add_git_project(repo, "main".into()).unwrap();
        let key = FactKey::workspace(&w, domux_core::facts::FACT_BRANCH);
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("feat/x", None)),
        });
        core.tick();
        let (answered, fact) = next_answer(&mut rx).await;
        assert_eq!(answered, key);
        assert_eq!(
            fact, None,
            "a provider that did not finish reports an absence, not the value it had before"
        );
        core.handle(CoreMsg::FactFetched {
            key: answered,
            fact,
        });
        assert_eq!(core.facts.get(&key), None, "and the fact is not frozen");
        core.tick();
        assert_eq!(next_answer(&mut rx).await.0, key);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "the target is looked at again rather than left in flight for the life of the server"
        );
    }

    /// What domux observed an hour ago is not what is true now.
    #[test]
    fn a_tick_drops_a_fact_past_its_time_to_live_and_says_it_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let key = FactKey::workspace(&first_workspace(&core), domux_core::facts::FACT_PR);
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        core.pending_events.clear();
        core.view_dirty = false;
        core.tick();
        assert_eq!(
            core.facts.get(&key).map(|f| f.text.as_str()),
            Some("PR#212"),
            "inside its ten minutes it stands"
        );
        assert_eq!(fact_events(&core), Vec::new());
        // The same number, fetched an hour before this server's clock reads.
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(stamped(
                "PR#212",
                Some(FactState::Open),
                "2026-09-04T13:32:00",
            )),
        });
        core.pending_events.clear();
        core.view_dirty = false;
        core.tick();
        assert_eq!(
            core.facts.get(&key),
            None,
            "an hour old open pull request is not drawn like one seen a second ago"
        );
        assert_eq!(
            fact_events(&core),
            vec![Event::FactUpdated {
                key,
                present: false
            }],
            "and the screen is told, or the number stays up until something else redraws"
        );
        assert!(core.view_dirty);
    }

    #[test]
    fn an_answer_about_a_workspace_that_is_gone_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let key = FactKey::workspace(&first_workspace(&core), domux_core::facts::FACT_PR);
        let project = core.model.projects[0].id.clone();
        core.model.remove_project(&project).unwrap();
        core.pending_events.clear();
        core.handle(CoreMsg::FactFetched {
            key: key.clone(),
            fact: Some(a_fact("PR#212", Some(FactState::Open))),
        });
        assert_eq!(
            core.facts.get(&key),
            None,
            "the workspace went away while its provider was running, so its answer is about nothing"
        );
        assert_eq!(
            fact_events(&core),
            Vec::new(),
            "and nothing announces a fact that was never recorded"
        );
    }

    // The working glyph and the pool of working words.
    //
    // Both live here rather than in `tests/agents_animation.rs`, because neither is visible
    // from a frame. A server with nothing working draws no glyph on any row, so its screen
    // stands still whether the core counted the tick or threw it away, and an identical
    // frame sends no diff: the whole cost of a missing guard is work nobody sees. The pool
    // is a count inside the core that no row shows either. `WorkingWords::in_use` is what
    // makes both observable, and this module is the only place that can read it.

    /// One hook payload from `pane`, through the handler the report subcommand reaches.
    fn hook(core: &mut Core, pane: &PaneId, event: &str) {
        hook_with(core, pane, event, None);
    }

    /// The same, carrying a transcript, which is what puts an entry in the recap cache: the
    /// handler reads the recap on `SessionStart`, `UserPromptSubmit` and `Stop`.
    fn hook_with(core: &mut Core, pane: &PaneId, event: &str, transcript: Option<&Path>) {
        let mut payload = serde_json::json!({"hook_event_name": event, "session_id": "c1"});
        if let Some(path) = transcript {
            payload["transcript_path"] = serde_json::json!(path);
        }
        let method = Method::from_request(
            "agent.report",
            serde_json::json!({"pane": pane, "kind": "claude", "payload": payload}),
        )
        .expect("agent.report takes these params");
        core.dispatch(method, None).expect("agent.report");
    }

    /// A transcript on disk for the recap reader to cache.
    fn a_transcript(dir: &Path) -> PathBuf {
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, "{}\n").expect("write the transcript");
        path
    }

    /// The view every frame builds, which is where a working agent takes its word. `render`
    /// builds one per frame and needs a client and a composed buffer; this is the half of it
    /// the pool turns on, and it is the same function.
    fn drawn(core: &mut Core) -> crate::render::agents_box::AgentsView {
        let now = core.deps.clock.now();
        agents_view(&core.model, &mut core.agents, &core.config.keymap, now)
    }

    /// A core and the pane its implicit workspace starts with, for a hook to report from.
    fn core_with_a_pane(dir: &Path) -> (Core, PaneId) {
        let core = core(dir);
        let pane = core
            .model
            .all_pane_ids()
            .first()
            .cloned()
            .expect("the implicit workspace starts with one pane");
        (core, pane)
    }

    /// A frame of animation turns the glyph and asks for the frame that draws it. The
    /// assertion is on the glyph the view carries rather than on the counter, so a count
    /// that no view reads would fail here.
    #[test]
    fn an_animation_tick_turns_the_glyph_while_an_agent_works() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook(&mut core, &pane, "UserPromptSubmit");
        let before = drawn(&mut core).glyph;
        core.view_dirty = false;

        core.handle(CoreMsg::AnimationTick);

        assert!(
            core.view_dirty,
            "the frame the new glyph is drawn in is asked for"
        );
        assert_ne!(drawn(&mut core).glyph, before, "and the glyph moved on");
    }

    /// And it costs a server with nothing working nothing at all. The ticker never starts and
    /// never stops (M3 plan assumption 35), so this guard is the only thing between an idle
    /// server and twelve and a half redraws a second for the rest of its life.
    ///
    /// The last three lines are what stop this passing on a server that never animates: the
    /// same core and the same record, working.
    #[test]
    fn an_animation_tick_leaves_a_server_with_nothing_working_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook(&mut core, &pane, "SessionStart");
        assert_eq!(core.model.agents.len(), 1, "there is a record to pass over");
        core.view_dirty = false;

        for _ in 0..crate::agents::labels::GLYPH_FRAMES.len() {
            core.handle(CoreMsg::AnimationTick);
        }

        assert_eq!(core.agents.glyph_tick, 0, "an idle record turns nothing");
        assert!(!core.view_dirty, "and asks for no frame");

        hook(&mut core, &pane, "UserPromptSubmit");
        core.view_dirty = false;
        core.handle(CoreMsg::AnimationTick);
        assert_eq!(
            core.agents.glyph_tick, 1,
            "the same core turns once it works"
        );
        assert!(core.view_dirty);
    }

    /// A compacting agent animates too: the glyph says something is happening, and compacting
    /// is something happening. It carries no working word, which is the other half of the same
    /// rule (a word is shown for `working` and for nothing else), and the two are asserted
    /// together so neither is mistaken for the other.
    #[test]
    fn a_compacting_agent_turns_the_glyph_and_carries_no_word() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook(&mut core, &pane, "UserPromptSubmit");
        assert!(!drawn(&mut core).agents[0].word.is_empty(), "working first");

        hook(&mut core, &pane, "PreCompact");
        core.view_dirty = false;
        core.handle(CoreMsg::AnimationTick);

        assert_eq!(
            core.agents.glyph_tick, 1,
            "compacting keeps the glyph turning"
        );
        assert!(core.view_dirty);
        assert_eq!(
            drawn(&mut core).agents[0].word,
            "",
            "and shows no working word"
        );
        assert_eq!(
            core.agents.words.in_use(),
            0,
            "which it gave back on the way"
        );
    }

    /// The gate in `agents_view`. `word_for` mutates, so a row that must not show a word must
    /// not ask for one either: without the gate every record would take a slot from a pool of
    /// 186 merely by being listed, and the animation lists them all twelve and a half times a
    /// second for as long as anything works.
    #[test]
    fn a_record_that_is_not_working_takes_no_word_from_the_pool() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook(&mut core, &pane, "SessionStart");

        // Twenty frames, which is under two seconds of the animation.
        for _ in 0..20 {
            let view = drawn(&mut core);
            assert_eq!(view.agents.len(), 1, "the idle record is listed");
            assert_eq!(view.agents[0].word, "", "and draws no word");
        }
        assert_eq!(core.agents.words.in_use(), 0, "so it took no slot");

        // The positive half, on the same record: a pool nothing ever reaches would pass the
        // lines above on its own.
        hook(&mut core, &pane, "UserPromptSubmit");
        assert!(!drawn(&mut core).agents[0].word.is_empty());
        assert_eq!(
            core.agents.words.in_use(),
            1,
            "a working record does take one"
        );
    }

    /// The hook path's release. A word is per working agent, and an agent a hook says has
    /// stopped is not one.
    #[test]
    fn a_stop_hook_gives_the_working_word_back_to_the_pool() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook(&mut core, &pane, "UserPromptSubmit");
        drawn(&mut core);
        assert_eq!(core.agents.words.in_use(), 1, "the working row took a word");

        hook(&mut core, &pane, "Stop");

        assert_eq!(
            core.agents.words.in_use(),
            0,
            "and gave it back on the stop"
        );
        drawn(&mut core);
        assert_eq!(
            core.agents.words.in_use(),
            0,
            "the next frame does not take it again"
        );
    }

    /// One hook payload from `pane` carrying `session`, for the two records that displace
    /// each other below.
    fn hook_session(core: &mut Core, pane: &PaneId, event: &str, session: Option<&str>) {
        let mut payload = serde_json::json!({ "hook_event_name": event });
        if let Some(id) = session {
            payload["session_id"] = serde_json::json!(id);
        }
        let method = Method::from_request(
            "agent.report",
            serde_json::json!({"pane": pane, "kind": "claude", "payload": payload}),
        )
        .expect("agent.report takes these params");
        core.dispatch(method, None).expect("agent.report");
    }

    /// A new session on a pane exits the record that was there, and that record was working a
    /// moment ago, so its word goes back.
    ///
    /// The handler used to free the word of the record the hook named and no other, which left
    /// the displaced one exited and still holding a slot. Nothing on the screen shows it: the
    /// row draws no word for an exited record either way. The pool is 186 words, so a server
    /// that runs long enough hands out a word another record already has.
    #[test]
    fn a_session_that_takes_a_pane_gives_back_the_word_of_the_one_it_displaced() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook_session(&mut core, &pane, "UserPromptSubmit", Some("c1"));
        drawn(&mut core);
        assert_eq!(
            core.agents.words.in_use(),
            1,
            "the first session took a word"
        );

        hook_session(&mut core, &pane, "UserPromptSubmit", Some("c2"));
        drawn(&mut core);

        let states: Vec<AgentState> = core.model.agents.iter().map(|a| a.state).collect();
        assert_eq!(
            states,
            vec![AgentState::Exited, AgentState::Working],
            "the pane changed hands, so one record exited and one is working"
        );
        assert_eq!(
            core.agents.words.in_use(),
            1,
            "one word for the one working record, and the displaced one gave its own back"
        );
    }

    /// The same when the pane changes hands between two kinds, which is the shape a reader
    /// meets: they close claude and start codex in the pane it was working in.
    ///
    /// A second test over the branch above rather than a duplicate of it. It arrives by a
    /// different route (the kinds differ, so the payload cannot be matched to the live record
    /// by session id) and on a different hook, and it is the sequence Task 17's review wrote
    /// out when it found the leak, so it is the one a reader will come looking for.
    #[test]
    fn a_different_kind_taking_over_a_pane_gives_back_the_working_word() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook_session(&mut core, &pane, "UserPromptSubmit", Some("c1"));
        drawn(&mut core);
        assert_eq!(
            core.agents.words.in_use(),
            1,
            "claude is working and has a word"
        );

        let method = Method::from_request(
            "agent.report",
            serde_json::json!({
                "pane": &pane,
                "kind": "codex",
                "payload": {"hook_event_name": "SessionStart", "session_id": "x1"},
            }),
        )
        .expect("agent.report takes these params");
        core.dispatch(method, None).expect("agent.report");
        drawn(&mut core);

        use domux_core::model::agent::AgentKind;
        let records: Vec<(AgentKind, AgentState)> = core
            .model
            .agents
            .iter()
            .map(|a| (a.kind, a.state))
            .collect();
        assert_eq!(
            records,
            vec![
                (AgentKind::Claude, AgentState::Exited),
                (AgentKind::Codex, AgentState::Idle)
            ],
            "codex took the pane and the claude record exited with it"
        );
        assert_eq!(
            core.agents.words.in_use(),
            0,
            "nothing is working, so no word is held"
        );
    }

    /// The same for the record a resume removes rather than exits. A session-less record the
    /// observer left on a pane is dropped when the session that owns that pane reports from
    /// it, and a dropped record frees its word like an exited one.
    #[test]
    fn a_resume_that_drops_a_placeholder_gives_back_the_word_it_held() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, first) = core_with_a_pane(dir.path());
        let client = attached(&mut core);
        core.dispatch(
            Method::from_request("pane.split", serde_json::json!({"dir": "right"}))
                .expect("pane.split takes these params"),
            Some(client),
        )
        .expect("pane.split");
        let second = core
            .model
            .all_pane_ids()
            .into_iter()
            .find(|p| p != &first)
            .expect("the split made a second pane");
        hook_session(&mut core, &first, "UserPromptSubmit", Some("c1"));
        // No session id, so this record is the placeholder the resume below drops.
        hook_session(&mut core, &second, "UserPromptSubmit", None);
        drawn(&mut core);
        assert_eq!(
            core.agents.words.in_use(),
            2,
            "two working records, two words"
        );

        hook_session(&mut core, &second, "UserPromptSubmit", Some("c1"));
        drawn(&mut core);

        assert_eq!(core.model.agents.len(), 1, "the placeholder was dropped");
        assert_eq!(
            core.agents.words.in_use(),
            1,
            "and it did not take a slot with it"
        );
    }

    /// The observer path's release, which is the other way out of `working`. Every record the
    /// observer exits is released in `Core::agents_changed`, whether the once-a-second pass
    /// found the process gone or the pane's child did, so this holds one site for both.
    #[test]
    fn a_pane_that_exits_gives_back_the_working_words_of_its_agents() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        hook(&mut core, &pane, "UserPromptSubmit");
        drawn(&mut core);
        assert_eq!(core.agents.words.in_use(), 1, "the working row took a word");

        core.handle(CoreMsg::PaneExited {
            pane: pane.clone(),
            status: Some(0),
        });

        assert_eq!(
            core.model.agents[0].state,
            AgentState::Exited,
            "the record went with the pane"
        );
        assert_eq!(core.agents.words.in_use(), 0, "and its word went with it");
    }

    /// A delete takes every record of the workspace, working ones included, so the words they
    /// hold have to go back by hand: `Model::remove_workspace` drops the records, and after
    /// that nothing can name the slots they held.
    #[test]
    fn deleting_a_workspace_gives_back_the_working_words_of_its_agents() {
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let workspace = a_slot(&mut core, dir.path());
        // A slot is registered without a tab. The invariant every dispatch runs on its way
        // out is what gives it one, and the pane comes with the tab.
        core.ensure_every_workspace_has_a_tab();
        let pane = core
            .model
            .workspace(&workspace)
            .and_then(|w| w.tabs.first())
            .and_then(|t| t.layout.pane_ids().first().cloned())
            .expect("the slot's tab starts with one pane");
        let transcript = a_transcript(dir.path());
        hook_with(&mut core, &pane, "UserPromptSubmit", Some(&transcript));
        drawn(&mut core);
        assert_eq!(core.agents.words.in_use(), 1, "the working row took a word");
        assert_eq!(
            core.agents.recaps.cached(),
            1,
            "and its transcript is cached"
        );

        core.workspace_deleted(
            None,
            workspace,
            "workspace-1".into(),
            "workspace-1".to_string(),
        )
        .expect("the workspace is deleted");

        assert!(
            core.model.agents.is_empty(),
            "the records went with the slot"
        );
        assert_eq!(core.agents.words.in_use(), 0, "and so did their words");
        assert_eq!(
            core.agents.recaps.cached(),
            0,
            "and their cached transcripts"
        );
    }

    /// Removing a project takes every record under it, so their caches go with them.
    ///
    /// The harsher of the two removal paths and the one worth a test of its own. A clear
    /// leaves its records behind as exited, so a word it missed is still reclaimable by a
    /// later dismiss; a project removal takes the record away, and nothing can ever name what
    /// it held again.
    #[test]
    fn removing_a_project_gives_back_the_caches_of_the_records_under_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        let transcript = a_transcript(dir.path());
        hook_with(&mut core, &pane, "UserPromptSubmit", Some(&transcript));
        drawn(&mut core);
        assert_eq!(core.agents.words.in_use(), 1, "the working row took a word");
        assert_eq!(
            core.agents.recaps.cached(),
            1,
            "and its transcript is cached"
        );
        let project = core.model.projects[0].id.clone();

        core.dispatch(
            Method::from_request(
                "project.remove",
                serde_json::json!({ "project": project.as_str(), "yes": true }),
            )
            .expect("project.remove takes these params"),
            None,
        )
        .expect("project.remove");

        assert!(core.model.agents.is_empty(), "the records went with it");
        assert_eq!(core.agents.words.in_use(), 0, "and so did their words");
        assert_eq!(
            core.agents.recaps.cached(),
            0,
            "and their cached transcripts"
        );
    }

    /// `api::agent::dismiss` gives back the word of the record it removes.
    ///
    /// The word is put in the pool here rather than by a hook, because no sequence of hooks
    /// can leave one for a dismiss to find: a record has to be exited before `dismiss_agent`
    /// will take it, and every way out of `working` frees the word on the way. There are three
    /// of them and the tests above hold all three: a hook that stops the agent, a pane that
    /// exits under it, and a report that takes its pane for another session. The third leaked
    /// a word until Task 17, which is why this is set by hand rather than driven by hooks, and
    /// also why the line is worth pinning: the pool is finite, an agent id is never reissued,
    /// and a slot leaked in it is leaked for the life of the server, so a later change that
    /// makes this path reachable must not depend on someone adding the release back.
    #[test]
    fn dismissing_a_record_gives_its_working_word_back_to_the_pool() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, pane) = core_with_a_pane(dir.path());
        let transcript = a_transcript(dir.path());
        hook_with(&mut core, &pane, "SessionStart", Some(&transcript));
        hook(&mut core, &pane, "SessionEnd");
        let id = core.model.agents[0].id.clone();
        assert_eq!(core.model.agents[0].state, AgentState::Exited);
        assert_eq!(core.agents.recaps.cached(), 1, "the transcript is cached");
        core.agents.words.word_for(&id);
        assert_eq!(core.agents.words.in_use(), 1);

        let method =
            Method::from_request("agent.dismiss", serde_json::json!({ "agent": id.as_str() }))
                .expect("agent.dismiss takes these params");
        core.dispatch(method, None).expect("agent.dismiss");

        assert!(core.model.agents.is_empty(), "the record is gone");
        assert_eq!(
            core.agents.words.in_use(),
            0,
            "and its word is back in the pool"
        );
        assert_eq!(
            core.agents.recaps.cached(),
            0,
            "and its transcript is forgotten"
        );
    }
}
