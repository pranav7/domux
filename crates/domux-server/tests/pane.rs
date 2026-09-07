use domux_core::ids::PaneId;
use domux_server::core::CoreMsg;
use domux_server::pane::{
    new_pane_emulator, FakeSpawner, PaneRuntime, PtySpawner, RealSpawner, SpawnRequest, PANE_TERM,
};
use domux_term::{GhosttyEmulator, Rgb, Size};
use std::path::PathBuf;
use std::time::Duration;

fn emulator(size: Size) -> GhosttyEmulator {
    new_pane_emulator(
        size,
        100,
        Rgb {
            r: 0xcd,
            g: 0xd6,
            b: 0xf4,
        },
        Rgb {
            r: 0x1e,
            g: 0x1e,
            b: 0x2e,
        },
    )
    .unwrap()
}

fn request(id: &str, command: &[&str], cwd: &str) -> SpawnRequest {
    SpawnRequest {
        pane: PaneId(id.into()),
        command: command.iter().map(|s| s.to_string()).collect(),
        cwd: PathBuf::from(cwd),
        env: vec![
            ("DOMUX_PANE".into(), id.into()),
            ("DOMUX_SOCKET".into(), "/tmp/x.sock".into()),
        ],
        size: Size { cols: 200, rows: 5 },
        term: PANE_TERM.into(),
    }
}

/// Kills the child on every exit path out of a test, a panic included, so a failing
/// assertion never leaves a process behind.
struct Reaped(PaneRuntime);

impl Drop for Reaped {
    fn drop(&mut self) {
        self.0.pty.kill();
    }
}

/// The exit code once the child has been reaped, polled until `deadline`. The reader thread
/// sees only EOF, and on Linux the master can reach EOF before the child is reaped, so a
/// single `try_wait` can honestly answer `None`. Polling waits for the fact instead of
/// racing it.
async fn exit_code_by(pane: &mut PaneRuntime, deadline: tokio::time::Instant) -> Option<i32> {
    loop {
        if let Some(code) = pane.pty.exit_status() {
            return Some(code);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn real_pty_delivers_output_env_and_exit_through_core_messages() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let dir = tempfile::tempdir().unwrap();
    let req = request(
        "p_0001",
        &[
            "sh",
            "-c",
            "printf \"%s %s $DOMUX_PANE\" \"$TERM\" \"$(pwd)\"; exit 3",
        ],
        dir.path().to_str().unwrap(),
    );
    let pty = RealSpawner.spawn(req, tx).unwrap();
    let mut pane = Reaped(PaneRuntime::new(
        PaneId("p_0001".into()),
        emulator(Size { cols: 200, rows: 5 }),
        pty,
    ));
    // The process inspector needs both of these: the master fd to ask who is in the
    // foreground, and the pid when it has to fall back to the child itself.
    assert!(pane.0.pty.pid().is_some());
    assert!(pane.0.pty.raw_fd().is_some());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let msg = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("message before deadline")
            .expect("open");
        match msg {
            CoreMsg::PaneOutput { bytes, .. } => pane.0.feed(&bytes),
            CoreMsg::PaneExited { status, .. } => {
                // The reader thread sees only EOF; the exit code comes from the handle.
                let status = match status {
                    Some(code) => Some(code),
                    None => exit_code_by(&mut pane.0, deadline).await,
                };
                pane.0.exited = Some(status);
                break;
            }
            _ => {}
        }
    }
    pane.0.snapshot();
    let row: String = pane.0.grid.row(0).iter().map(|c| c.text.as_str()).collect();
    let expected = format!(
        "xterm-256color {} p_0001",
        dir.path().canonicalize().unwrap().display()
    );
    assert!(
        row.trim_end() == expected
            || row.trim_end() == format!("xterm-256color {} p_0001", dir.path().display()),
        "{row:?}"
    );
    assert_eq!(pane.0.exited, Some(Some(3)));
}

#[tokio::test(flavor = "current_thread")]
async fn fake_spawner_records_requests_and_captures_writes() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let fake = FakeSpawner::default();
    let pty = fake.spawn(request("p_0002", &[], "/tmp"), tx).unwrap();
    let mut pane = PaneRuntime::new(
        PaneId("p_0002".into()),
        emulator(Size { cols: 200, rows: 5 }),
        pty,
    );
    // No process, so there is no pid and no fd: absent, not a zero.
    assert_eq!(pane.pty.pid(), None);
    assert_eq!(pane.pty.raw_fd(), None);
    pane.write(b"ls\r");
    assert_eq!(fake.requests().len(), 1);
    assert_eq!(fake.requests()[0].cwd, PathBuf::from("/tmp"));
    assert_eq!(fake.written(&PaneId("p_0002".into())), b"ls\r".to_vec());
    pane.feed(b"hello");
    assert!(pane.dirty);
    pane.snapshot();
    assert!(!pane.dirty);
    assert_eq!(pane.grid.cell(0, 0).text, "h");
}

#[tokio::test(flavor = "current_thread")]
async fn feed_writes_the_emulators_replies_back_to_the_pty() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let fake = FakeSpawner::default();
    let pty = fake.spawn(request("p_0003", &[], "/tmp"), tx).unwrap();
    let mut pane = PaneRuntime::new(
        PaneId("p_0003".into()),
        emulator(Size { cols: 200, rows: 5 }),
        pty,
    );
    // A cursor position report is owed to the program, not drawn.
    pane.feed(b"ab\x1b[6n");
    assert_eq!(
        fake.written(&PaneId("p_0003".into())),
        b"\x1b[1;3R".to_vec()
    );
    pane.snapshot();
    assert_eq!(pane.grid.cell(0, 0).text, "a");
    // Draining is once only: a second feed with nothing owed writes nothing more.
    pane.feed(b"c");
    assert_eq!(
        fake.written(&PaneId("p_0003".into())),
        b"\x1b[1;3R".to_vec()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn resize_to_the_current_size_leaves_the_pane_clean() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let fake = FakeSpawner::default();
    let pty = fake.spawn(request("p_0004", &[], "/tmp"), tx).unwrap();
    let mut pane = PaneRuntime::new(
        PaneId("p_0004".into()),
        emulator(Size { cols: 200, rows: 5 }),
        pty,
    );
    pane.snapshot();
    pane.resize(Size { cols: 200, rows: 5 });
    assert!(!pane.dirty);
    pane.resize(Size { cols: 80, rows: 24 });
    assert!(pane.dirty);
    assert_eq!(pane.size(), Size { cols: 80, rows: 24 });
    pane.snapshot();
    assert_eq!(pane.grid.size(), Size { cols: 80, rows: 24 });
}
