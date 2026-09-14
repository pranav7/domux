//! One pane at runtime: its PTY, its emulator, its grid. The PTY sits behind `PtySpawner`
//! so resume tests can run without processes. Lifted from the M0 pane spike. The spike's
//! sample collector is not lifted: it kept every sample in growing vectors, which suits a
//! spike that exits and not a daemon (M0 outcome 5).

use crate::copy_mode::CopyMode;
use crate::core::CoreMsg;
use anyhow::{Context, Result};
use domux_core::ids::PaneId;
use domux_term::{Emulator, EmulatorConfig, GhosttyEmulator, Grid, Rgb, Size};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc::Sender;

/// `TERM` inside every pane (M0 assumption 9).
pub const PANE_TERM: &str = "xterm-256color";
const READ_CHUNK: usize = 64 * 1024;

/// The names a pane keeps from the server's own environment. Everything else is dropped.
///
/// The server is a daemon, and its environment is an accident of whichever shell started it.
/// Start it from a shell that had `CLAUDE_CODE_USE_BEDROCK=1` set and, without this list, every
/// pane opened for the rest of the day runs a shell that has it too, so `claude` in a pane
/// silently reaches a different provider than `claude` in a terminal. `TMUX` and `TERM_PROGRAM`
/// are the same defect in an older form: a pane that reports it is inside tmux, or inside
/// whichever terminal happened to launch the server. Keeping a named few ends the class;
/// dropping names one at a time only ever catches the ones already found.
///
/// A pane runs a login shell, so everything a shell config exports is set again inside the
/// pane. This list carries only what a login shell cannot work out for itself, and what a pane
/// started with an explicit command needs.
const KEPT: &[&str] = &[
    // Who the pane belongs to, what it runs, and where it writes.
    "HOME",
    "USER",
    "LOGNAME",
    "PATH",
    "SHELL",
    "TMPDIR",
    // How it formats text and time.
    "LANG",
    "TZ",
    // The agent a pane pushes with, and the display it draws on.
    "SSH_AUTH_SOCK",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    // Where anything reading the XDG names keeps its config, data and cache.
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    // The macOS login session. `SECURITYSESSIONID` is how a process reaches the keychain,
    // which is where the credentials of the programs a pane runs are kept, so a pane without
    // it could be handed a shell that cannot log in to anything.
    "SECURITYSESSIONID",
    "__CF_USER_TEXT_ENCODING",
];

/// Locale is a family, not a name: `LC_ALL`, `LC_CTYPE`, `LC_TIME` and the rest.
const KEPT_PREFIX: &str = "LC_";

/// Whether a pane keeps `name` from the server's environment (`KEPT`).
pub fn pane_keeps(name: &str) -> bool {
    KEPT.contains(&name) || name.starts_with(KEPT_PREFIX)
}

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
    /// Stops the reader between two reads. It sends `CoreMsg::ReaderPaused` once every byte
    /// it read has been sent, and reads nothing more until `resume_reader`: what the program
    /// prints meanwhile waits in the PTY (decision 0045).
    fn pause_reader(&mut self);
    fn resume_reader(&mut self);
    /// Gives the PTY up without closing it or signalling its program, for an upgrade to carry
    /// (decision 0045). The reader stops for good. `None` when there is nothing to carry: the
    /// child has already exited.
    fn hand_over(self: Box<Self>) -> Option<HandedPty>;
}

/// What crosses an upgrade of one pane's PTY: the master descriptor, still open, and the
/// process id of the program on the other side, still the server's child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandedPty {
    pub fd: RawFd,
    pub pid: Option<u32>,
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

    /// Wraps a PTY an earlier server handed over (decision 0045) in the handle `spawn` would
    /// have given it, and starts its reader. A spawner that never hands a PTY over cannot take
    /// one back, which is every spawner a test writes for itself.
    fn adopt(
        &self,
        pane: PaneId,
        pty: HandedPty,
        tx: Sender<CoreMsg>,
    ) -> Result<Box<dyn PtyHandle>> {
        let _ = (pane, pty, tx);
        anyhow::bail!("this spawner cannot adopt a PTY")
    }
}

pub struct RealSpawner;

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
        // A pane's environment is built, not inherited (`KEPT`): the server's own is whatever
        // the shell that started the daemon happened to hold.
        cmd.env_clear();
        for (name, value) in std::env::vars() {
            if pane_keeps(&name) {
                cmd.env(name, value);
            }
        }
        cmd.env("TERM", &req.term);
        cmd.env("COLORTERM", "truecolor");
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
        let pid = child
            .process_id()
            .context("the spawned child has no process id")?;
        let fd = pair
            .master
            .as_raw_fd()
            .context("the PTY has no master descriptor")?;
        // The handle keeps its own copy of the master, so portable-pty's can close with it.
        // Dropping `child` neither kills nor waits for the program: the handle waits for it
        // by its process id from here on, which is also all an adopted pane has.
        let master = dup_cloexec(fd).context("copy the PTY's master descriptor")?;
        drop(pair.master);
        drop(child);
        Ok(Box::new(UnixPty::start(req.pane, master, pid, tx)?))
    }

    fn adopt(
        &self,
        pane: PaneId,
        pty: HandedPty,
        tx: Sender<CoreMsg>,
    ) -> Result<Box<dyn PtyHandle>> {
        let pid = pty
            .pid
            .context("a handed over PTY without a process id cannot be adopted")?;
        // Safe: the descriptor was handed over open and nothing else in this process owns it.
        let master = unsafe { std::fs::File::from_raw_fd(pty.fd) };
        set_cloexec(pty.fd, true).context("mark the adopted descriptor close-on-exec")?;
        Ok(Box::new(UnixPty::start(pane, master, pid, tx)?))
    }
}

/// A pane's PTY as a descriptor and a process id, which is what a spawned pane and an adopted
/// one have in common. Everything is a system call on those two, so the handle an upgrade
/// hands over is the handle it takes back.
struct UnixPty {
    master: std::fs::File,
    pid: u32,
    /// Set once the child has been reaped. Its process id can by then belong to an unrelated
    /// process, so nothing signals or waits for it after that.
    status: Option<i32>,
    reader: Arc<ReaderGate>,
}

impl UnixPty {
    fn start(
        pane: PaneId,
        master: std::fs::File,
        pid: u32,
        tx: Sender<CoreMsg>,
    ) -> Result<UnixPty> {
        let reader = master
            .try_clone()
            .context("copy the PTY's master descriptor for its reader")?;
        let gate = Arc::new(ReaderGate::new().context("make the reader's wake pipe")?);
        let thread_gate = gate.clone();
        thread::Builder::new()
            .name(format!("pty-reader-{pane}"))
            .spawn(move || read_pane(pane, reader, &thread_gate, &tx))?;
        Ok(UnixPty {
            master,
            pid,
            status: None,
            reader: gate,
        })
    }
}

impl PtyHandle for UnixPty {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.master.write_all(bytes)?;
        self.master.flush()
    }

    fn resize(&mut self, size: Size) -> io::Result<()> {
        let ws = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // Safe: `ws` is a valid winsize for the duration of the call.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// What portable-pty's kill did: a hangup, a quarter of a second for the program to take
    /// it, then a kill, and a wait either way so nothing is left a zombie.
    fn kill(&mut self) {
        if self.exit_status().is_some() {
            return;
        }
        let pid = self.pid as libc::pid_t;
        unsafe { libc::kill(pid, libc::SIGHUP) };
        for attempt in 0..5 {
            if attempt > 0 {
                thread::sleep(std::time::Duration::from_millis(50));
            }
            if self.exit_status().is_some() {
                return;
            }
        }
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let mut raw = 0;
        if unsafe { libc::waitpid(pid, &mut raw, 0) } == pid {
            self.status = Some(exit_code(raw));
        }
    }

    fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    fn raw_fd(&self) -> Option<RawFd> {
        Some(self.master.as_raw_fd())
    }

    fn exit_status(&mut self) -> Option<i32> {
        if self.status.is_none() {
            let pid = self.pid as libc::pid_t;
            let mut raw = 0;
            if unsafe { libc::waitpid(pid, &mut raw, libc::WNOHANG) } == pid {
                self.status = Some(exit_code(raw));
            }
        }
        self.status
    }

    fn pause_reader(&mut self) {
        self.reader.set(ReaderState::Pausing);
    }

    fn resume_reader(&mut self) {
        self.reader.set(ReaderState::Running);
    }

    fn hand_over(mut self: Box<Self>) -> Option<HandedPty> {
        self.reader.set(ReaderState::Stopped);
        if self.exit_status().is_some() {
            return None;
        }
        let UnixPty { master, pid, .. } = *self;
        Some(HandedPty {
            fd: master.into_raw_fd(),
            pid: Some(pid),
        })
    }
}

/// The exit code `waitpid` reported, the way portable-pty reported it: the code a program
/// exited with, and 1 for one a signal ended.
fn exit_code(raw: libc::c_int) -> i32 {
    if libc::WIFEXITED(raw) {
        libc::WEXITSTATUS(raw)
    } else {
        1
    }
}

/// A copy of `fd` that closes on exec, so no program the server starts inherits a pane's PTY.
fn dup_cloexec(fd: RawFd) -> io::Result<std::fs::File> {
    // Safe: F_DUPFD_CLOEXEC returns a new descriptor this process owns, or -1.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(copy) })
}

/// Sets or clears close-on-exec on `fd`. An upgrade clears it on what it carries just before
/// the exec, and sets it again on whatever the exec did not take (decision 0045).
pub fn set_cloexec(fd: RawFd, on: bool) -> io::Result<()> {
    // Safe: F_GETFD and F_SETFD read and write one flag on a descriptor and touch no memory.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let flags = if on {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReaderState {
    Running,
    /// Asked to stop between two reads, and not yet stopped.
    Pausing,
    /// Stopped between two reads, and said so.
    Paused,
    /// Gone for good, without a word to the core: the PTY has been handed over.
    Stopped,
}

/// Where a reader thread and the core meet. The thread blocks in `poll` on the PTY and on a
/// pipe; the core writes a byte to the pipe whenever it changes the state, so a thread waiting
/// for output wakes at once rather than on a timer, and an idle pane costs nothing.
struct ReaderGate {
    state: Mutex<ReaderState>,
    changed: std::sync::Condvar,
    wake_read: std::fs::File,
    wake_write: std::fs::File,
}

impl ReaderGate {
    fn new() -> io::Result<ReaderGate> {
        let mut fds = [0; 2];
        // Safe: `fds` has room for the two descriptors `pipe` writes.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // Safe: `pipe` returned two descriptors this process now owns.
        let (wake_read, wake_write) = unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        };
        for fd in fds {
            set_cloexec(fd, true)?;
            // Safe: F_SETFL on a descriptor this process owns.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(ReaderGate {
            state: Mutex::new(ReaderState::Running),
            changed: std::sync::Condvar::new(),
            wake_read,
            wake_write,
        })
    }

    fn set(&self, to: ReaderState) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // A stopped reader stays stopped, and one that has already said it paused is not
        // asked again: the core would wait for a second message that never comes.
        match (*state, to) {
            (ReaderState::Stopped, _) => return,
            (ReaderState::Paused, ReaderState::Pausing) => return,
            _ => *state = to,
        }
        drop(state);
        self.changed.notify_all();
        // A full pipe already holds a wake-up, so a failed write loses nothing.
        let _ = (&self.wake_write).write(&[1]);
    }

    /// Called by the reader between two reads. Answers false when the reader should end.
    fn proceed(&self, pane: &PaneId, tx: &Sender<CoreMsg>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state == ReaderState::Pausing {
            *state = ReaderState::Paused;
            drop(state);
            if tx
                .blocking_send(CoreMsg::ReaderPaused { pane: pane.clone() })
                .is_err()
            {
                return false;
            }
            state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        }
        while *state == ReaderState::Paused {
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        *state != ReaderState::Stopped
    }

    fn drain_wakes(&self) {
        let mut buf = [0u8; 64];
        while matches!((&self.wake_read).read(&mut buf), Ok(n) if n > 0) {}
    }
}

/// One pane's reader: output to the core as `PaneOutput`, then `PaneExited` once the program
/// has closed its side. It stops only between two reads (`ReaderGate::proceed`), so a byte is
/// either sent to the core or still in the PTY, never held here.
fn read_pane(pane: PaneId, mut reader: std::fs::File, gate: &ReaderGate, tx: &Sender<CoreMsg>) {
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        if !gate.proceed(&pane, tx) {
            return;
        }
        let mut fds = [
            libc::pollfd {
                fd: reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: gate.wake_read.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // Safe: `fds` is a valid array of two pollfd for the duration of the call.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if fds[1].revents != 0 {
            gate.drain_wakes();
            continue;
        }
        if fds[0].revents == 0 {
            continue;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx
                    .blocking_send(CoreMsg::PaneOutput {
                        pane: pane.clone(),
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
    let _ = tx.blocking_send(CoreMsg::PaneExited { pane, status: None });
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

/// The first descriptor number the fake spawner hands out. It is not a real descriptor and
/// nothing passes it to the kernel: only `FakeInspector` is ever asked about one, and it
/// exists so a test can tell one fake pane's PTY from another's.
pub const FAKE_PTY_FD_BASE: RawFd = 100;

/// No process. Records spawn requests and everything written, so tests can assert on them.
#[derive(Default)]
pub struct FakeSpawner {
    requests: Mutex<Vec<SpawnRequest>>,
    written: WriteLog,
    /// Every spawn from now on fails, for a test that needs a pane with no process behind it.
    refusing: std::sync::atomic::AtomicBool,
    /// Every PTY an upgrade handed back, in the order they were adopted.
    adopted: Mutex<Vec<(PaneId, HandedPty)>>,
}

impl FakeSpawner {
    pub fn requests(&self) -> Vec<SpawnRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Every PTY this spawner adopted from an upgrade's handover, in order.
    pub fn adopted(&self) -> Vec<(PaneId, HandedPty)> {
        self.adopted.lock().unwrap().clone()
    }

    /// Makes every later spawn fail, which is the state a `terminal.shell` that cannot start
    /// puts the real spawner in: the pane record exists and no runtime does. `spawn_pane` logs
    /// such a failure and carries on, so a test can ask what the rest of the system does about
    /// a pane nothing is running.
    pub fn refuse_spawns(&self) {
        self.refusing
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The descriptor this spawner gave the pane's PTY, which is what `FakeInspector::set_for`
    /// keys on. The pane's last spawn answers: a pane whose process was replaced is running
    /// the newer one.
    pub fn raw_fd(&self, pane: &PaneId) -> Option<RawFd> {
        let requests = self.requests.lock().unwrap();
        let index = requests.iter().rposition(|r| &r.pane == pane)?;
        Some(FAKE_PTY_FD_BASE + index as RawFd)
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
    fd: RawFd,
    /// Where a pause is acknowledged. A fake pane has no reader thread, so it has read
    /// everything the moment it is asked to stop.
    tx: Sender<CoreMsg>,
}

impl PtySpawner for FakeSpawner {
    fn spawn(&self, req: SpawnRequest, tx: Sender<CoreMsg>) -> Result<Box<dyn PtyHandle>> {
        let pane = req.pane.clone();
        let index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(req);
            requests.len() - 1
        };
        if self.refusing.load(std::sync::atomic::Ordering::SeqCst) {
            anyhow::bail!("the fake spawner was told to refuse");
        }
        Ok(Box::new(FakePty {
            pane,
            written: self.written.clone(),
            fd: FAKE_PTY_FD_BASE + index as RawFd,
            tx,
        }))
    }

    fn adopt(
        &self,
        pane: PaneId,
        pty: HandedPty,
        tx: Sender<CoreMsg>,
    ) -> Result<Box<dyn PtyHandle>> {
        self.adopted.lock().unwrap().push((pane.clone(), pty));
        Ok(Box::new(FakePty {
            pane,
            written: self.written.clone(),
            fd: pty.fd,
            tx,
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

    /// The number the spawner assigned, not a descriptor the kernel knows: it tells this
    /// pane's PTY from another's so the fake inspector can answer for one pane at a time.
    fn raw_fd(&self) -> Option<RawFd> {
        Some(self.fd)
    }

    fn exit_status(&mut self) -> Option<i32> {
        None
    }

    fn pause_reader(&mut self) {
        let _ = self.tx.try_send(CoreMsg::ReaderPaused {
            pane: self.pane.clone(),
        });
    }

    fn resume_reader(&mut self) {}

    fn hand_over(self: Box<Self>) -> Option<HandedPty> {
        Some(HandedPty {
            fd: self.fd,
            pid: None,
        })
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
    /// after it leaves nothing behind (decision 0014).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_keeps_what_a_login_shell_cannot_work_out_for_itself() {
        for name in [
            "HOME",
            "PATH",
            "SHELL",
            "TMPDIR",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "SSH_AUTH_SOCK",
            "SECURITYSESSIONID",
        ] {
            assert!(pane_keeps(name), "a pane needs {name}");
        }
    }

    #[test]
    fn a_pane_drops_the_session_state_of_the_shell_that_started_the_server() {
        for name in [
            // The shell that started the daemon was a Claude Code session on Bedrock. Every
            // one of these reached the pane before the keep list, and the first two are what
            // made `claude` in a pane a different provider than `claude` in a terminal.
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_MESSAGING_SOCKET",
            "CLAUDECODE",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "AWS_PROFILE",
            "AWS_REGION",
            // The older form of the same defect: a pane that reports the multiplexer or the
            // terminal that launched the server rather than the one it is in.
            "TMUX",
            "TMUX_PANE",
            "TERM_PROGRAM",
            "TERM_PROGRAM_VERSION",
        ] {
            assert!(!pane_keeps(name), "a pane must not inherit {name}");
        }
    }

    /// The pane sets these itself, after the keep list, so keeping them would be reading the
    /// server's answer to a question the pane has already answered.
    #[test]
    fn a_pane_drops_the_names_it_sets_for_itself() {
        for name in ["TERM", "COLORTERM", "DOMUX_PANE", "DOMUX_SOCKET"] {
            assert!(!pane_keeps(name), "the pane sets {name} itself");
        }
    }
}
