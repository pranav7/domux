use domux_core::ids::PaneId;
use domux_server::core::CoreMsg;
use domux_server::pane::{
    new_pane_emulator, FakeSpawner, PaneRuntime, PtySpawner, RealSpawner, SpawnRequest,
    FAKE_PTY_FD_BASE, PANE_TERM,
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
    // No process, so there is no pid: absent, not a zero.
    assert_eq!(pane.pty.pid(), None);
    // A descriptor there is, and it is not one the kernel knows: it is a number that tells
    // this pane's PTY from the next one's, which is what `FakeInspector::set_for` keys on. The
    // handle and the spawner agree about it, so a test can ask either one.
    assert_eq!(pane.pty.raw_fd(), Some(FAKE_PTY_FD_BASE));
    assert_eq!(
        fake.raw_fd(&PaneId("p_0002".into())),
        Some(FAKE_PTY_FD_BASE)
    );
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

/// The fake's descriptors exist to tell one pane's PTY from another's, which is what lets a
/// test put a different process in front of each pane.
#[tokio::test(flavor = "current_thread")]
async fn every_fake_pane_gets_a_descriptor_of_its_own() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let fake = FakeSpawner::default();
    let first = fake
        .spawn(request("p_0005", &[], "/tmp"), tx.clone())
        .unwrap();
    let second = fake.spawn(request("p_0006", &[], "/tmp"), tx).unwrap();
    assert_eq!(first.raw_fd(), Some(FAKE_PTY_FD_BASE));
    assert_eq!(second.raw_fd(), Some(FAKE_PTY_FD_BASE + 1));
    assert_eq!(fake.raw_fd(&PaneId("p_0005".into())), first.raw_fd());
    assert_eq!(fake.raw_fd(&PaneId("p_0006".into())), second.raw_fd());
    // A pane the spawner never saw has no descriptor at all.
    assert_eq!(fake.raw_fd(&PaneId("p_0009".into())), None);
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

/// Opening a terminal runs a login shell, and a pane is a terminal. A shell config keeps its
/// PATH, its aliases and its functions in the login profile - zsh reads `.zprofile` only for a
/// login shell - so a pane that skipped it would hand back a shell the user does not
/// recognise: their own aliases would be gone. This asserts the profile is read, which is the
/// fact a user would notice, rather than the `-` marker on argv[0] that produces it.
#[tokio::test(flavor = "current_thread")]
async fn a_pane_with_no_command_runs_the_shell_through_its_login_profile() {
    let dir = tempfile::tempdir().unwrap();
    // `sh` reads `$HOME/.profile` when it is a login shell and not otherwise. Pointing HOME at
    // a temporary directory keeps the test off the real one.
    std::fs::write(
        dir.path().join(".profile"),
        "printf 'PROFILE_READ=[%s]' \"$0\"\nexit 0\n",
    )
    .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let mut req = request("p_0001", &[], dir.path().to_str().unwrap());
    req.env.push(("SHELL".into(), "/bin/sh".into()));
    req.env
        .push(("HOME".into(), dir.path().to_str().unwrap().to_string()));
    let pty = RealSpawner.spawn(req, tx).unwrap();
    let mut pane = Reaped(PaneRuntime::new(
        PaneId("p_0001".into()),
        emulator(Size { cols: 200, rows: 5 }),
        pty,
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let msg = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("message before deadline")
            .expect("open");
        match msg {
            CoreMsg::PaneOutput { bytes, .. } => pane.0.feed(&bytes),
            CoreMsg::PaneExited { .. } => break,
            _ => {}
        }
    }
    pane.0.snapshot();
    let row: String = pane.0.grid.row(0).iter().map(|c| c.text.as_str()).collect();
    assert!(
        row.starts_with("PROFILE_READ="),
        "the login profile did not run, row was {row:?}"
    );
}

/// What the *server* asks for, which is the half a spawner test cannot see. This fix was
/// written and proved against `RealSpawner` while the server was still sending a command, so
/// the login shell never ran: the request has to say "the shell" for the spawner to run one.
#[tokio::test]
async fn the_server_asks_for_the_shell_rather_than_naming_it_as_a_command() {
    use domux_core::config::Config;
    use domux_server::testing::Harness;

    let mut cfg = Config::default();
    cfg.terminal.shell = Some("/bin/dash".into());
    let h = Harness::start(cfg, 40, 10).await;
    let reqs = h.spawner.as_ref().expect("fake spawner").requests();
    let first = reqs.first().expect("the first pane was spawned");
    assert!(
        first.command.is_empty(),
        "a named command is not a login shell: {:?}",
        first.command
    );
    assert_eq!(
        first
            .env
            .iter()
            .find(|(k, _)| k == "SHELL")
            .map(|(_, v)| v.as_str()),
        Some("/bin/dash"),
        "the configured shell reaches the spawner, and the pane, through the environment"
    );
}

/// A shell that cannot be run is refused by name. `new_default_prog` would otherwise fall back
/// to the password database and quietly run a different shell, which turns a typo in
/// `terminal.shell` into a pane that works and a setting that appears to do nothing.
#[test]
fn a_shell_that_cannot_be_run_is_refused_rather_than_swapped() {
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let dir = tempfile::tempdir().unwrap();
    let mut req = request("p_0001", &[], dir.path().to_str().unwrap());
    req.env.push(("SHELL".into(), "/no/such/shell".to_string()));
    let Err(err) = RealSpawner.spawn(req, tx) else {
        panic!("a shell that is not there must not spawn");
    };
    let said = format!("{err:#}");
    assert!(said.contains("/no/such/shell"), "{said}");
    assert!(said.contains("terminal.shell"), "{said}");

    // A directory is not a shell either, and is the case a bare existence check would pass.
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut req = request("p_0002", &[], dir.path().to_str().unwrap());
    req.env
        .push(("SHELL".into(), dir.path().to_str().unwrap().to_string()));
    let Err(err) = RealSpawner.spawn(req, tx) else {
        panic!("a directory must not spawn");
    };
    assert!(
        format!("{err:#}").contains("not an executable file"),
        "{err:#}"
    );
}

/// The keep list, in a real pane rather than in the predicate. `CARGO_PKG_NAME` stands in for
/// the session state of the shell that started the server: cargo sets it in this test process,
/// which is the server for the length of this test, and nothing sets it in a pane.
#[tokio::test(flavor = "current_thread")]
async fn a_pane_does_not_inherit_the_servers_own_environment() {
    assert!(
        std::env::var("CARGO_PKG_NAME").is_ok(),
        "the fixture needs a variable this process has and a pane must not"
    );
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let dir = tempfile::tempdir().unwrap();
    let req = request(
        "p_0005",
        &[
            "sh",
            "-c",
            // Three answers in one row: what leaked, what the pane was told, and whether the
            // pane can still find anything to run.
            "printf \"[%s][%s][%s]\" \"$CARGO_PKG_NAME\" \"$DOMUX_PANE\" \"${PATH:+PATH}\"",
        ],
        dir.path().to_str().unwrap(),
    );
    let pty = RealSpawner.spawn(req, tx).unwrap();
    let mut pane = Reaped(PaneRuntime::new(
        PaneId("p_0005".into()),
        emulator(Size { cols: 200, rows: 5 }),
        pty,
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let msg = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("message before deadline")
            .expect("open");
        match msg {
            CoreMsg::PaneOutput { bytes, .. } => pane.0.feed(&bytes),
            CoreMsg::PaneExited { .. } => break,
            _ => {}
        }
    }
    pane.0.snapshot();
    let row: String = pane.0.grid.row(0).iter().map(|c| c.text.as_str()).collect();
    assert_eq!(row.trim_end(), "[][p_0005][PATH]");
}
