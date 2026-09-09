//! `server start|stop|restart|status|log`, and the hidden `run` that is the server.

use super::{call, call_as, print_line, socket};
use anyhow::Context;
use clap::{Args, Subcommand};
use domux_client::control;
use domux_core::api::ServerInfo;
use domux_core::names::{BIN_NAME, PRODUCT_NAME};
use domux_core::paths;
use domux_server::pane::RealSpawner;
use domux_server::process::RealInspector;
use domux_server::{load_config, CoreDeps, Server, ServerOptions, SystemClock};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long `start` and `stop` wait for the socket to appear or go. Long enough for a
/// loaded machine, short enough that a wedged server is reported rather than waited on.
const SETTLE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(25);

#[derive(Args)]
pub struct ServerCmd {
    #[command(subcommand)]
    pub action: ServerAction,
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Start the server in the background
    Start,
    /// Stop the server; panes end, structure is saved
    Stop,
    /// Stop, then start
    Restart,
    /// Show version, socket, state directory, config and clients
    Status,
    /// Print the path of the server log
    Log,
    /// Run the server in the foreground (used by start)
    #[command(hide = true)]
    Run,
}

pub async fn run(cmd: ServerCmd) -> anyhow::Result<()> {
    match cmd.action {
        ServerAction::Start => start(Announce::Yes).await,
        ServerAction::Stop => stop().await,
        ServerAction::Restart => restart().await,
        ServerAction::Status => status().await,
        ServerAction::Log => super::print_line(&paths::log_file().display().to_string()),
        ServerAction::Run => run_server().await,
    }
}

/// Whether the start reports itself. `server start` typed on its own says what it did,
/// because the command has nothing else to show for itself. The start behind an attach says
/// nothing: the screen that follows is the answer, and telling the reader to attach would
/// name an action already underway.
#[derive(Clone, Copy, PartialEq)]
pub enum Announce {
    Yes,
    No,
}

/// Spawns `server run` in its own session, detached from this terminal, and waits
/// for the socket. Called by `start` and by attach when the socket is absent.
pub async fn start(announce: Announce) -> anyhow::Result<()> {
    let socket = socket();
    if control::is_live(&socket).await {
        if announce == Announce::Yes {
            eprintln!("The server is already running.");
        }
        return Ok(());
    }
    // The child's stderr is the log, so a start that failed says why whatever failed: an
    // error before the server's own logging is up, a panic, or anything the runtime prints.
    // Nothing here depends on the server reaching its logging code.
    let log = paths::log_file();
    let sink = open_log(&log)?;
    let written_before = sink.metadata().map(|m| m.len()).unwrap_or(0);
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["server", "run"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(sink));
    // Safe: `setsid` has no preconditions in a freshly forked child, and both calls in the
    // closure are async-signal-safe.
    unsafe {
        cmd.pre_exec(|| enter_new_session(|| libc::setsid()));
    }
    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + SETTLE;
    while !control::is_live(&socket).await {
        // A server that has already exited will never open the socket, so say so now
        // rather than after the whole wait.
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "The server exited ({status}) before it opened {}. {}",
                socket.display(),
                reason(&log, written_before)
            );
        }
        if Instant::now() > deadline {
            anyhow::bail!(
                "The server did not open {} within {} seconds. {}",
                socket.display(),
                SETTLE.as_secs(),
                reason(&log, written_before)
            );
        }
        tokio::time::sleep(POLL).await;
    }
    if announce == Announce::Yes {
        eprintln!(
            "Server started (pid {}). Attach with {BIN_NAME}.",
            child.id()
        );
    }
    Ok(())
}

/// Stop, then start. A server that was not running is not news to someone who asked for a
/// restart, so the stop is quiet about it and only the start reports itself.
async fn restart() -> anyhow::Result<()> {
    stop_if_running().await?;
    start(Announce::Yes).await
}

/// The log, opened for appending, so the child's stderr and the server's own tracing land in
/// one file in the order they happened. The directory is made here rather than left to the
/// child, since a directory that cannot be made is the failure this call is about to report.
fn open_log(log: &Path) -> anyhow::Result<File> {
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .with_context(|| format!("open {}", log.display()))
}

/// What this start wrote to the log, as the sentence that follows the failure. Only what was
/// appended after `offset` is read, so an older run's last line is never reported as this
/// one's reason. A log with nothing in it is said to be empty and the reader is sent
/// somewhere that will answer, because "read this file" is no next action when the file
/// holds nothing (principle 9).
fn reason(log: &Path, offset: u64) -> String {
    match last_line_after(log, offset) {
        Some(line) => format!("It said: {line}. There may be more in {}.", log.display()),
        None => format!(
            "It wrote nothing to {}. Run {BIN_NAME} server run to see the failure on this terminal.",
            log.display()
        ),
    }
}

fn last_line_after(log: &Path, offset: u64) -> Option<String> {
    let mut file = File::open(log).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
}

pub async fn stop() -> anyhow::Result<()> {
    if !stop_if_running().await? {
        eprintln!("The server is not running.");
    }
    Ok(())
}

/// Stops a running server and waits for the socket to go. Answers whether there was one to
/// stop, so `restart` can be quiet about a server that was already stopped.
async fn stop_if_running() -> anyhow::Result<bool> {
    let socket = socket();
    if !control::is_live(&socket).await {
        return Ok(false);
    }
    call("server.stop", serde_json::json!({})).await?;
    let deadline = Instant::now() + SETTLE;
    while control::is_live(&socket).await {
        if Instant::now() > deadline {
            anyhow::bail!(
                "The server acknowledged the stop but is still listening after {} seconds. Read {}.",
                SETTLE.as_secs(),
                paths::log_file().display()
            );
        }
        tokio::time::sleep(POLL).await;
    }
    eprintln!("Server stopped.");
    Ok(true)
}

/// The report is data, so it goes to stdout. It is read out of the typed result rather
/// than poked at by field name, so a server whose answer this build cannot read fails
/// rather than printing a line of question marks.
async fn status() -> anyhow::Result<()> {
    let info: ServerInfo = call_as("server.info", serde_json::json!({})).await?;
    print_line(&format!(
        "Server {PRODUCT_NAME} {} (pid {}), started {}",
        info.version,
        info.pid,
        human_time(&info.started_at)
    ))?;
    print_line(&format!("Socket  {}", info.socket.display()))?;
    print_line(&format!("State   {}", info.state_dir.display()))?;
    print_line(&config_line(
        &info.config_file,
        info.config_error.as_deref(),
        info.config_file.exists(),
    ))?;
    // After the config line, because that is the question it answers: the file can say one
    // leader while the server runs another, and nothing else on screen would tell you which
    // key to press.
    print_line(&format!("Leader  {}", info.leader))?;
    print_line(&format!("Clients: {}", info.clients.len()))
}

/// Puts the child in its own session, so it outlives the terminal that started it. A child
/// that could not leave this terminal's session would die with the terminal, so a failure
/// ends the start rather than passing for one.
fn enter_new_session(setsid: impl Fn() -> i32) -> std::io::Result<()> {
    if setsid() == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The config line of the report. A file that is not there must not read as one that is: on a
/// first boot there is no domux.toml, and a bare path invites the reader to open a file that
/// is not there, or to assume it is and wonder why an edit had no effect. The server and the
/// CLI share a filesystem - the socket is a unix socket - so the path it names is one this
/// process can ask about.
fn config_line(path: &Path, error: Option<&str>, exists: bool) -> String {
    match (error, exists) {
        // A file that failed to load says so first: whether it is there is the smaller half
        // of that answer, and the error names it.
        (Some(e), _) => format!("Config  {} (not applied: {e})", path.display()),
        (None, false) => format!("Config  {} (not created yet)", path.display()),
        (None, true) => format!("Config  {}", path.display()),
    }
}

/// The server's start time as a person reads it. The clock's own value carries microseconds,
/// which read as machine output in a human report. A value this build cannot parse is printed
/// exactly as it arrived rather than guessed at.
fn human_time(started_at: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(started_at) {
        Ok(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        Err(_) => started_at.to_string(),
    }
}

/// The pane ids of two servers must not collide, so the seed is drawn fresh.
fn id_seed() -> anyhow::Result<u64> {
    id_seed_from(Path::new("/dev/urandom"))
}

/// A machine that cannot answer for eight random bytes is reported, never seeded with zeroes:
/// two servers seeded the same way would hand out the same pane ids.
fn id_seed_from(source: &Path) -> anyhow::Result<u64> {
    let mut bytes = [0u8; 8];
    File::open(source)
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| anyhow::anyhow!("could not read {} for an id seed: {e}", source.display()))?;
    Ok(u64::from_le_bytes(bytes))
}

/// The server process. Logs to the state directory, serves until SIGTERM or SIGINT or a
/// `server.stop`, then persists and exits.
async fn run_server() -> anyhow::Result<()> {
    let state_dir = paths::state_dir();
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("create {}", state_dir.display()))?;
    domux_server::log::init(&paths::log_file())?;
    // The log exists from here on, so a failure is written there as well as returned. That
    // covers `server run` typed by hand, where stderr is a terminal and not the log.
    serve(state_dir)
        .await
        .inspect_err(|e| tracing::error!("the server stopped: {e:#}"))
}

async fn serve(state_dir: PathBuf) -> anyhow::Result<()> {
    let opts = ServerOptions {
        socket_path: socket(),
        state_dir,
        config: load_config(&paths::config_file()),
        project_root: std::env::current_dir()?,
        providers: domux_server::facts::default_providers(),
        deps: CoreDeps {
            spawner: Arc::new(RealSpawner),
            inspector: Arc::new(RealInspector),
            clock: Arc::new(SystemClock),
            id_seed: id_seed()?,
        },
    };
    let handle = Server::start(opts).await?;
    tracing::info!(
        "{PRODUCT_NAME} {} listening on {}",
        domux_core::VERSION,
        handle.socket_path.display()
    );
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    // `server.stop` makes the core return and drop its receiver, so the same wait covers
    // the API path and the two signals.
    let core_tx = handle.core_tx.clone();
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = sigint.recv() => {}
        _ = core_tx.closed() => {}
    }
    // Every exit path waits for the persistence task to flush, and it is what removes the
    // socket. Persistence is debounced, so returning straight from the wait would lose the
    // last state.json write.
    handle.stop().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_seed_is_the_bytes_it_read_and_a_source_it_cannot_read_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let eight = dir.path().join("eight");
        std::fs::write(&eight, [1u8, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(id_seed_from(&eight).unwrap(), 1);
        let short = dir.path().join("short");
        std::fs::write(&short, [1u8, 2, 3]).unwrap();
        assert!(id_seed_from(&short).is_err(), "three bytes are not a seed");
        assert!(id_seed_from(&dir.path().join("gone")).is_err());
    }

    #[test]
    fn a_session_the_child_could_not_leave_ends_the_start() {
        assert!(enter_new_session(|| -1).is_err());
        assert!(enter_new_session(|| 4711).is_ok());
    }

    #[test]
    fn the_config_line_says_when_the_file_is_not_there_yet() {
        let path = Path::new("/home/u/.config/domux2/domux.toml");
        assert_eq!(
            config_line(path, None, false),
            "Config  /home/u/.config/domux2/domux.toml (not created yet)"
        );
        assert_eq!(
            config_line(path, None, true),
            "Config  /home/u/.config/domux2/domux.toml"
        );
        assert_eq!(
            config_line(path, Some("domux.toml line 3: unknown key clock"), true),
            "Config  /home/u/.config/domux2/domux.toml (not applied: domux.toml line 3: unknown key clock)"
        );
    }

    #[test]
    fn a_start_time_is_shown_to_the_second_and_an_unreadable_one_exactly_as_it_arrived() {
        assert_eq!(
            human_time("2026-09-07T21:07:01.738699+01:00"),
            "2026-09-07 21:07:01"
        );
        assert_eq!(human_time("whenever"), "whenever");
    }

    #[test]
    fn the_reason_is_what_this_start_appended_not_an_older_run() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("server.log");
        std::fs::write(&log, "an older run said this\n").unwrap();
        let offset = std::fs::metadata(&log).unwrap().len();
        assert!(
            reason(&log, offset).contains("It wrote nothing to"),
            "{}",
            reason(&log, offset)
        );
        assert!(!reason(&log, offset).contains("an older run"));
        std::fs::write(
            &log,
            "an older run said this\ncreate /x: Permission denied\n\n",
        )
        .unwrap();
        let said = reason(&log, offset);
        assert!(
            said.starts_with("It said: create /x: Permission denied."),
            "{said}"
        );
    }

    #[test]
    fn the_reason_names_the_log_when_there_is_no_log_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("gone.log");
        let said = reason(&log, 0);
        assert!(said.contains("It wrote nothing to"), "{said}");
        assert!(said.contains("gone.log"), "{said}");
        assert!(said.contains("domux2 server run"), "{said}");
    }
}
