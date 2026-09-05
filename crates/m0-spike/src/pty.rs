//! One pane's PTY: spawn the child, read its output on a thread, write input, resize, exit.

use anyhow::{Context, Result};
use domux_term::{Emulator, Size};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::thread;
use tokio::sync::mpsc::Sender;

pub enum PaneMsg {
    Output { pane: usize, bytes: Vec<u8> },
    Exited { pane: usize },
}

pub struct Pane {
    pub id: usize,
    pub emulator: Box<dyn Emulator>,
    pub dirty: bool,
    pub exited: bool,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    responses: Vec<u8>,
}

const READ_CHUNK: usize = 64 * 1024;

/// Spawns `command` (argv; empty means `$SHELL` or `/bin/sh`) in a new PTY of `size` with
/// `TERM=term`, and starts a reader thread that sends output and the exit into `tx`.
pub fn spawn(
    id: usize,
    command: &[String],
    size: Size,
    term: &str,
    emulator: Box<dyn Emulator>,
    tx: Sender<PaneMsg>,
) -> Result<Pane> {
    let pty = native_pty_system();
    let pair = pty.openpty(pty_size(size)).context("openpty")?;
    let mut cmd = if command.is_empty() {
        CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()))
    } else {
        let mut c = CommandBuilder::new(&command[0]);
        c.args(&command[1..]);
        c
    };
    cmd.env("TERM", term);
    cmd.env("COLORTERM", "truecolor");
    cmd.env_remove("TMUX");
    cmd.env_remove("TMUX_PANE");
    cmd.env_remove("TERM_PROGRAM");
    cmd.env_remove("TERM_PROGRAM_VERSION");
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
    let child = pair.slave.spawn_command(cmd).context("spawn child")?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().context("clone reader")?;
    let writer = pair.master.take_writer().context("take writer")?;

    thread::Builder::new()
        .name(format!("pty-reader-{id}"))
        .spawn(move || {
            let mut buf = vec![0u8; READ_CHUNK];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx
                            .blocking_send(PaneMsg::Output {
                                pane: id,
                                bytes: buf[..n].to_vec(),
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    // Linux returns EIO on the master once the child closes its side.
                    Err(_) => break,
                }
            }
            let _ = tx.blocking_send(PaneMsg::Exited { pane: id });
        })?;

    Ok(Pane {
        id,
        emulator,
        dirty: true,
        exited: false,
        master: pair.master,
        writer,
        child,
        responses: Vec::new(),
    })
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Pane {
    /// Feeds output into the emulator and writes any replies straight back to the child.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.emulator.feed(bytes);
        self.dirty = true;
        self.emulator.take_responses(&mut self.responses);
        if !self.responses.is_empty() {
            let _ = self.writer.write_all(&self.responses);
            let _ = self.writer.flush();
            self.responses.clear();
        }
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, size: Size) -> Result<()> {
        self.master.resize(pty_size(size)).context("resize pty")?;
        self.emulator.resize(size);
        self.dirty = true;
        Ok(())
    }

    pub fn mark_exited(&mut self) {
        self.exited = true;
        let _ = self.child.try_wait();
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
