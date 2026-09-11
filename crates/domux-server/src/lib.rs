//! The domux server: one core task owns the model, the panes and the clients.

pub mod agents;
pub mod api;
pub mod client;
pub mod command;
pub mod copy_mode;
pub mod core;
pub mod facts;
pub mod git;
pub mod input;
pub mod link;
pub mod log;
pub mod migrate;
pub mod mouse;
pub mod pane;
pub mod persist;
pub mod process;
pub mod render;
pub mod socket;
pub mod stay_awake;
pub mod subprocess;
pub mod testing;
pub mod toast;
pub mod worktree_conf;

use crate::core::{Core, CoreMsg};
use crate::facts::FactProvider;
use crate::pane::PtySpawner;
use crate::process::ProcessInspector;
use anyhow::Context;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use domux_core::config::{Config, ConfigError};
use domux_core::facts::{Fact, FactKey};
use domux_core::ids::PaneId;
use domux_core::keymap::Keymap;
use domux_core::model::Model;
use domux_term::Size;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// The configuration as loaded: what applies, and what went wrong. A bad file never blocks
/// startup (architecture spec section 9); `error` carries the line for the notice.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: Config,
    pub keymap: Keymap,
    pub error: Option<ConfigError>,
    pub warnings: Vec<String>,
}

pub fn load_config(path: &Path) -> LoadedConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return LoadedConfig {
                path: path.to_path_buf(),
                config: Config::default(),
                keymap: Keymap::defaults(),
                // A file that could not be opened has no line to send the reader to.
                error: Some(ConfigError {
                    line: None,
                    column: None,
                    message: format!("could not read the file: {e}"),
                }),
                warnings: Vec::new(),
            };
        }
    };
    match Config::parse(&text) {
        Ok(parsed) => {
            let mut warnings: Vec<String> = parsed.warnings.into_iter().map(|w| w.0).collect();
            match Keymap::from_config(&parsed.config.keys) {
                Ok((keymap, kw)) => {
                    warnings.extend(kw.into_iter().map(|w| w.0));
                    LoadedConfig {
                        path: path.to_path_buf(),
                        config: parsed.config,
                        keymap,
                        error: None,
                        warnings,
                    }
                }
                // The file parsed, so the toml has no error and there is no span to take a
                // line from: the message names the setting instead. Absent, not line 1.
                Err(message) => LoadedConfig {
                    path: path.to_path_buf(),
                    config: Config::default(),
                    keymap: Keymap::defaults(),
                    error: Some(ConfigError {
                        line: None,
                        column: None,
                        message,
                    }),
                    warnings,
                },
            }
        }
        Err(error) => LoadedConfig {
            path: path.to_path_buf(),
            config: Config::default(),
            keymap: Keymap::defaults(),
            error: Some(error),
            warnings: Vec::new(),
        },
    }
}

/// Wall-clock time, behind a trait so frames in tests show a fixed clock.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Local>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Local> {
        Local::now()
    }
}

pub struct FixedClock(pub DateTime<Local>);
impl FixedClock {
    /// `"2026-09-04T14:32:00"` in local time.
    pub fn at(s: &str) -> FixedClock {
        let naive =
            NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").expect("fixed clock format");
        FixedClock(
            Local
                .from_local_datetime(&naive)
                .single()
                .expect("unambiguous local time"),
        )
    }
}
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Local> {
        self.0
    }
}

/// Hands a link to whatever the desktop opens it with, behind a trait so no test opens a
/// browser. What may be handed over at all is `link`'s to decide, not this one's: an
/// implementation here does what it is told (decision record 0024).
pub trait Opener: Send + Sync {
    /// `Ok` means the opener was started, not that anything was displayed: the program it
    /// hands to reports to the desktop and not back here.
    fn open(&self, target: &str) -> Result<(), String>;
}

/// macOS's `open`, which is what a terminal there hands a link to.
pub struct SystemOpener;
impl Opener for SystemOpener {
    fn open(&self, target: &str) -> Result<(), String> {
        // `--` so a target that begins with a dash is an argument and never a flag. `link`
        // has already refused everything but an http or https URL and a path that exists, so
        // this is the second of the two guards rather than the only one.
        let out = std::process::Command::new("open")
            .arg("--")
            .arg(target)
            .output()
            .map_err(|e| format!("could not run open: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(&out.stderr);
        let last = said.lines().rev().find(|l| !l.trim().is_empty());
        Err(match last {
            Some(line) => line.trim().to_string(),
            None => format!("open exited {}", out.status),
        })
    }
}

/// What the core needs from the outside world. Tests pass fakes.
#[derive(Clone)]
pub struct CoreDeps {
    pub spawner: Arc<dyn PtySpawner>,
    pub inspector: Arc<dyn ProcessInspector>,
    pub clock: Arc<dyn Clock>,
    pub opener: Arc<dyn Opener>,
    /// Runs the programs that are not a pane's own. Stay awake starts and stops its holder
    /// through this, so a test records those calls rather than making them.
    pub runner: Arc<dyn crate::command::CommandRunner>,
    pub id_seed: u64,
    /// The operating system, as `std::env::consts::OS` spells it. Given rather than read, so
    /// a test can ask what this server does on a machine it is not running on.
    pub platform: String,
}

pub struct ServerOptions {
    pub socket_path: PathBuf,
    pub state_dir: PathBuf,
    pub config: LoadedConfig,
    /// The implicit plain-folder project (M1). Ignored when `state.json` already has one.
    pub project_root: PathBuf,
    pub deps: CoreDeps,
    /// Who observes the facts. A real server passes `facts::default_providers()`; a test
    /// passes its own list, so no test shells out to git or `gh`.
    pub providers: Vec<Arc<dyn FactProvider>>,
}

pub struct ServerHandle {
    pub core_tx: mpsc::Sender<CoreMsg>,
    pub socket_path: PathBuf,
    /// The model as of the end of the core's last batch. The core writes it; everyone else
    /// clones what they need out of it, so no reader ever holds the core's own state.
    pub snapshot: Arc<Mutex<Model>>,
    /// Each live pane's emulator size, which is what its program believes its screen is.
    /// The Model does not carry it - a pane's size lives in its runtime, not in the state
    /// that persists - so it is published beside the model rather than inside it. Written
    /// just before `snapshot` in the same batch, so a pane visible in the model always has
    /// a size here.
    pub pane_sizes: Arc<Mutex<HashMap<PaneId, Size>>>,
    /// Every fact the core currently holds, published beside `snapshot` for the same
    /// reason: the harness (and, later, a control API method) reads it rather than
    /// reaching into the core task, which owns all mutable state.
    pub facts: Arc<Mutex<HashMap<FactKey, Fact>>>,
    core: tokio::task::JoinHandle<()>,
    persist: tokio::task::JoinHandle<()>,
    listener: tokio::task::JoinHandle<()>,
    tick: tokio::task::JoinHandle<()>,
    animation: tokio::task::JoinHandle<()>,
}

impl ServerHandle {
    /// Persists, closes the PTYs, removes the socket. Returns once `state.json` is written.
    pub async fn stop(self) {
        let _ = self.core_tx.send(CoreMsg::Shutdown).await;
        let _ = self.core.await;
        let _ = self.persist.await;
        self.listener.abort();
        self.tick.abort();
        self.animation.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

pub struct Server;

impl Server {
    pub async fn start(opts: ServerOptions) -> anyhow::Result<ServerHandle> {
        std::fs::create_dir_all(&opts.state_dir)
            .with_context(|| format!("create {}", opts.state_dir.display()))?;
        let (core_tx, core_rx) = mpsc::channel::<CoreMsg>(1024);
        let (persist_tx, persist_rx) = mpsc::channel(64);
        let state_file = opts.state_dir.join("state.json");
        let persist = tokio::spawn(persist::spawn(state_file.clone(), persist_rx));
        let socket_path = opts.socket_path.clone();
        let snapshot = Arc::new(Mutex::new(Model::new(opts.deps.id_seed)));
        let pane_sizes = Arc::new(Mutex::new(HashMap::new()));
        let facts = Arc::new(Mutex::new(HashMap::new()));
        let core = Core::new(
            opts,
            core_tx.clone(),
            persist_tx,
            &state_file,
            snapshot.clone(),
            pane_sizes.clone(),
            facts.clone(),
        )?;
        let listener = socket::listen(&socket_path, core_tx.clone()).await?;
        let tick_tx = core_tx.clone();
        let tick = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                if tick_tx.send(CoreMsg::Tick).await.is_err() {
                    break;
                }
            }
        });
        // The working glyph, one tick every 70 ms. It runs from here to shutdown whether or
        // not anything is working: the core is what decides that, so there is no timer to
        // start and stop and no state about it to get wrong (M3 plan assumption 35). A tick
        // the core was too busy to take is skipped rather than queued, so a server that falls
        // behind resumes the animation instead of replaying the frames it missed.
        let animation_tx = core_tx.clone();
        let animation = tokio::spawn(async move {
            let mut interval = tokio::time::interval(crate::agents::labels::ANIMATION_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if animation_tx.send(CoreMsg::AnimationTick).await.is_err() {
                    break;
                }
            }
        });
        let core = tokio::spawn(core.run(core_rx));
        Ok(ServerHandle {
            core_tx,
            socket_path,
            snapshot,
            pane_sizes,
            facts,
            core,
            persist,
            listener,
            tick,
            animation,
        })
    }
}
