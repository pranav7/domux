//! One pane at runtime: its PTY, its emulator, its grid. The PTY sits behind `PtySpawner`
//! so resume tests can run without processes. Lifted from the M0 pane spike. The spike's
//! sample collector is not lifted: it kept every sample in growing vectors, which suits a
//! spike that exits and not a daemon (M0 outcome 5).

use crate::copy_mode::CopyMode;
use crate::core::CoreMsg;
use anyhow::{Context, Result};
use domux_core::ids::PaneId;
use domux_term::{Emulator, EmulatorConfig, GhosttyEmulator, Grid, Rgb, Size};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc::Sender;

/// `TERM` inside every pane (M0 assumption 9).
pub const PANE_TERM: &str = "xterm-256color";
const READ_CHUNK: usize = 64 * 1024;

/// The one place the server builds a pane's emulator. There is one implementation
/// (docs/decisions/0001-terminal-emulator.md), so this returns it by value and the feed path
/// has no dispatch.
pub fn new_pane_emulator(
    size: Size,
    scrollback_lines: usize,
    default_fg: Rgb,
    default_bg: Rgb,
) -> Result<GhosttyEmulator, String> {
    GhosttyEmulator::new(EmulatorConfig {
        size,
        scrollback_lines,
        default_fg,
        default_bg,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    pub pane: PaneId,
    /// argv. Empty means the shell named by `SHELL` in `env`, run as a *login* shell.
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub size: Size,
    pub term: String,
}

pub trait PtyHandle: Send {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn resize(&mut self, size: Size) -> io::Result<()>;
    fn kill(&mut self);
    fn pid(&self) -> Option<u32>;
    fn raw_fd(&self) -> Option<RawFd>;
    /// The child's exit code once it has exited, else `None`.
    fn exit_status(&mut self) -> Option<i32>;
}

/// Refuses a shell the pane could not run, naming it and the setting that names it
/// (principle 9). Without this the spawn would quietly succeed on a different shell.
fn executable(shell: &Path) -> Result<()> {
    let meta = std::fs::metadata(shell).with_context(|| {
        format!(
            "shell {} cannot be run; check terminal.shell",
            shell.display()
        )
    })?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        anyhow::bail!(
            "shell {} is not an executable file; check terminal.shell",
            shell.display()
        );
    }
    Ok(())
}

pub trait PtySpawner: Send + Sync {
    fn spawn(&self, req: SpawnRequest, tx: Sender<CoreMsg>) -> Result<Box<dyn PtyHandle>>;
}

pub struct RealSpawner;

struct RealPty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl PtySpawner for RealSpawner {
    fn spawn(&self, req: SpawnRequest, tx: Sender<CoreMsg>) -> Result<Box<dyn PtyHandle>> {
        let pty = native_pty_system();
        let pair = pty.openpty(pty_size(req.size)).context("openpty")?;
        // A pane with no command is a terminal, and opening a terminal runs a *login* shell:
        // that is where a shell config puts its PATH, its aliases and its functions (zsh reads
        // `.zprofile` only for a login shell). A pane that skipped them would hand the user a
        // shell they do not recognise.
        //
        // `new_default_prog` is the only login form `CommandBuilder` has: it prefixes argv[0]
        // with `-`, the way every terminal emulator does. It takes the shell from `SHELL` in
        // the command's environment, which is why the caller puts it there. It would fall back
        // to the password database for a shell it cannot execute, and that would hide a typo
        // in `terminal.shell` behind a shell that happens to work, so that is refused here.
        let mut cmd = match req.command.split_first() {
            Some((program, args)) => {
                let mut c = CommandBuilder::new(program);
                c.args(args);
                c
            }
            None => {
                let shell = req
                    .env
                    .iter()
                    .find(|(k, _)| k == "SHELL")
                    .map(|(_, v)| v.as_str())
                    .context("a pane with no command needs SHELL in its environment")?;
                executable(Path::new(shell))?;
                CommandBuilder::new_default_prog()
            }
        };
        cmd.env("TERM", &req.term);
        cmd.env("COLORTERM", "truecolor");
        for name in ["TMUX", "TMUX_PANE", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"] {
            cmd.env_remove(name);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        let cwd = if req.cwd.is_dir() {
            req.cwd.clone()
        } else {
            std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/"))
        };
        cmd.cwd(cwd);
        let child = pair.slave.spawn_command(cmd).context("spawn child")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;
        let id = req.pane.clone();
        thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn(move || {
                let mut buf = vec![0u8; READ_CHUNK];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx
                                .blocking_send(CoreMsg::PaneOutput {
                                    pane: id.clone(),
                                    bytes: buf[..n].to_vec(),
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        // Linux returns EIO once the child closes its side.
                        Err(_) => break,
                    }
                }
                let _ = tx.blocking_send(CoreMsg::PaneExited {
                    pane: id,
                    status: None,
                });
            })?;
        Ok(Box::new(RealPty {
            master: pair.master,
            writer,
            child,
        }))
    }
}

impl PtyHandle for RealPty {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    fn resize(&mut self, size: Size) -> io::Result<()> {
        self.master
            .resize(pty_size(size))
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn kill(&mut self) {
        // A child that has already exited has been reaped, and its pid can by then belong to
        // an unrelated process: portable-pty signals the pid without checking, so the check
        // is here. Killing twice is a no-op rather than a signal to a stranger.
        if self.exit_status().is_some() {
            return;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    fn raw_fd(&self) -> Option<RawFd> {
        self.master.as_raw_fd()
    }

    fn exit_status(&mut self) -> Option<i32> {
        self.child
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.exit_code() as i32)
    }
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Every write into the fake PTYs one `FakeSpawner` handed out, in the order they arrived.
/// Shared, so the spawner can still read a handle it has given away.
type WriteLog = Arc<Mutex<Vec<(PaneId, Vec<u8>)>>>;

/// No process. Records spawn requests and everything written, so tests can assert on them.
#[derive(Default)]
pub struct FakeSpawner {
    requests: Mutex<Vec<SpawnRequest>>,
    written: WriteLog,
    /// Every spawn from now on fails, for a test that needs a pane with no process behind it.
    refusing: std::sync::atomic::AtomicBool,
}

impl FakeSpawner {
    pub fn requests(&self) -> Vec<SpawnRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Makes every later spawn fail, which is the state a `terminal.shell` that cannot start
    /// puts the real spawner in: the pane record exists and no runtime does. `spawn_pane` logs
    /// such a failure and carries on, so a test can ask what the rest of the system does about
    /// a pane nothing is running.
    pub fn refuse_spawns(&self) {
        self.refusing
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn written(&self, pane: &PaneId) -> Vec<u8> {
        self.written
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == pane)
            .flat_map(|(_, b)| b.iter().copied())
            .collect()
    }
}

struct FakePty {
    pane: PaneId,
    written: WriteLog,
}

impl PtySpawner for FakeSpawner {
    fn spawn(&self, req: SpawnRequest, _tx: Sender<CoreMsg>) -> Result<Box<dyn PtyHandle>> {
        let pane = req.pane.clone();
        self.requests.lock().unwrap().push(req);
        if self.refusing.load(std::sync::atomic::Ordering::SeqCst) {
            anyhow::bail!("the fake spawner was told to refuse");
        }
        Ok(Box::new(FakePty {
            pane,
            written: self.written.clone(),
        }))
    }
}

impl PtyHandle for FakePty {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.written
            .lock()
            .unwrap()
            .push((self.pane.clone(), bytes.to_vec()));
        Ok(())
    }

    fn resize(&mut self, _size: Size) -> io::Result<()> {
        Ok(())
    }

    fn kill(&mut self) {}

    fn pid(&self) -> Option<u32> {
        None
    }

    fn raw_fd(&self) -> Option<RawFd> {
        None
    }

    fn exit_status(&mut self) -> Option<i32> {
        None
    }
}

pub struct PaneRuntime {
    pub id: PaneId,
    pub emulator: GhosttyEmulator,
    pub grid: Grid,
    pub dirty: bool,
    /// `Some(status)` once the child exited; the inner `None` means the status is unknown.
    pub exited: Option<Option<i32>>,
    pub pty: Box<dyn PtyHandle>,
    pub copy: Option<CopyMode>,
    /// Where the left button went down inside this pane, in the pane's own cells, while it is
    /// still held. The drag that follows anchors its selection there, and a press with no drag
    /// after it leaves nothing behind (decision 0013).
    pub pressed_at: Option<(u16, u16)>,
    responses: Vec<u8>,
}

impl PaneRuntime {
    pub fn new(id: PaneId, emulator: GhosttyEmulator, pty: Box<dyn PtyHandle>) -> PaneRuntime {
        let grid = Grid::new(emulator.size());
        PaneRuntime {
            id,
            emulator,
            grid,
            dirty: true,
            exited: None,
            pty,
            copy: None,
            pressed_at: None,
            responses: Vec::new(),
        }
    }

    /// Feeds output and writes any replies straight back to the child.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.emulator.feed(bytes);
        self.dirty = true;
        self.emulator.take_responses(&mut self.responses);
        if !self.responses.is_empty() {
            let _ = self.pty.write(&self.responses);
            self.responses.clear();
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if let Err(e) = self.pty.write(bytes) {
            tracing::debug!(pane = %self.id, "write to pty failed: {e}");
        }
    }

    pub fn resize(&mut self, size: Size) {
        if size == self.emulator.size() {
            return;
        }
        let _ = self.pty.resize(size);
        self.emulator.resize(size);
        // Copy mode is measured in the pane's screen, so it follows the screen: a cursor left
        // outside it would stop being drawn, and `$` would stop meaning the last column.
        if let Some(copy) = self.copy.as_mut() {
            copy.resized(size);
        }
        self.dirty = true;
    }

    pub fn size(&self) -> Size {
        self.emulator.size()
    }

    /// Refreshes `grid` from the emulator (at the copy mode offset when in copy mode).
    pub fn snapshot(&mut self) {
        // The offset is clamped here as well as on every key, because the history can shrink
        // without a key being pressed: a program switching to the alternate screen takes it
        // to none, and the viewport has to come back to the screen with it.
        let history = crate::copy_mode::history(self);
        match self.copy.as_mut() {
            Some(copy) => {
                copy.offset = copy.offset.min(history);
                self.emulator.snapshot_grid_at(copy.offset, &mut self.grid);
            }
            None => self.emulator.snapshot_grid(&mut self.grid),
        }
        self.dirty = false;
    }
}
