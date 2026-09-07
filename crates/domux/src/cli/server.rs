//! `server start|stop|restart|status|log`, and the hidden `run` that is the server.

use super::{call, call_as, socket};
use clap::{Args, Subcommand};
use domux_client::control;
use domux_core::api::ServerInfo;
use domux_core::names::{BIN_NAME, PRODUCT_NAME};
use domux_core::paths;
use domux_server::pane::RealSpawner;
use domux_server::process::RealInspector;
use domux_server::{load_config, CoreDeps, Server, ServerOptions, SystemClock};
use std::io::Read;
use std::os::unix::process::CommandExt;
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
        ServerAction::Start => start().await,
        ServerAction::Stop => stop().await,
        ServerAction::Restart => {
            stop().await?;
            start().await
        }
        ServerAction::Status => status().await,
        ServerAction::Log => {
            println!("{}", paths::log_file().display());
            Ok(())
        }
        ServerAction::Run => run_server().await,
    }
}

/// Spawns `server run` in its own session, detached from this terminal, and waits
/// for the socket. Called by `start` and by attach when the socket is absent.
pub async fn start() -> anyhow::Result<()> {
    let socket = socket();
    if control::is_live(&socket).await {
        eprintln!("The server is already running.");
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["server", "run"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Safe: `setsid` has no preconditions in a freshly forked child, and both calls in the
    // closure are async-signal-safe. A child that could not leave this terminal's session
    // would die with the terminal, so the failure ends the start rather than hiding.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + SETTLE;
    while !control::is_live(&socket).await {
        // A server that has already exited will never open the socket, so say so now
        // rather than after the whole wait.
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "The server exited ({status}) before it opened {}. Read {} for the reason.",
                socket.display(),
                paths::log_file().display()
            );
        }
        if Instant::now() > deadline {
            anyhow::bail!(
                "The server did not open {} within {} seconds. Read {} for the reason.",
                socket.display(),
                SETTLE.as_secs(),
                paths::log_file().display()
            );
        }
        tokio::time::sleep(POLL).await;
    }
    eprintln!(
        "Server started (pid {}). Attach with {BIN_NAME}.",
        child.id()
    );
    Ok(())
}

pub async fn stop() -> anyhow::Result<()> {
    let socket = socket();
    if !control::is_live(&socket).await {
        eprintln!("The server is not running.");
        return Ok(());
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
    Ok(())
}

/// The report is data, so it goes to stdout. It is read out of the typed result rather
/// than poked at by field name, so a server whose answer this build cannot read fails
/// rather than printing a line of question marks.
async fn status() -> anyhow::Result<()> {
    let info: ServerInfo = call_as("server.info", serde_json::json!({})).await?;
    println!(
        "Server {PRODUCT_NAME} {} (pid {}), started {}",
        info.version, info.pid, info.started_at
    );
    println!("Socket  {}", info.socket.display());
    println!("State   {}", info.state_dir.display());
    match &info.config_error {
        Some(e) => println!("Config  {} (not applied: {e})", info.config_file.display()),
        None => println!("Config  {}", info.config_file.display()),
    }
    println!("Clients: {}", info.clients.len());
    Ok(())
}

/// The pane ids of two servers must not collide, so the seed is drawn fresh. A machine
/// that cannot answer for eight random bytes is reported, never seeded with zeroes.
fn id_seed() -> anyhow::Result<u64> {
    let mut bytes = [0u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| anyhow::anyhow!("could not read /dev/urandom for an id seed: {e}"))?;
    Ok(u64::from_le_bytes(bytes))
}

/// The server process. Logs to the state directory, serves until SIGTERM or SIGINT or a
/// `server.stop`, then persists and exits.
async fn run_server() -> anyhow::Result<()> {
    let state_dir = paths::state_dir();
    std::fs::create_dir_all(&state_dir)?;
    domux_server::log::init(&paths::log_file())?;
    let opts = ServerOptions {
        socket_path: socket(),
        state_dir,
        config: load_config(&paths::config_file()),
        project_root: std::env::current_dir()?,
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
