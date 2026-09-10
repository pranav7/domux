//! The domux V2 binary: attach with no arguments; everything else is a socket client.

mod cli;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = domux_core::names::BIN_NAME,
    version = domux_core::VERSION,
    about = "A terminal multiplexer for engineers who direct AI agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Attach to the server, starting it first when it is not running
    Attach,
    /// Start, stop, restart or inspect the server
    Server(cli::server::ServerCmd),
    /// Reload domux.toml
    Config(cli::config::ConfigCmd),
    /// Call an API method: api <method> [json params]; api schema prints the schema
    Api(cli::api::ApiCmd),
    /// Tabs: create, name, clear-name, close, select
    Tab(cli::tab::TabCmd),
    /// Panes: split, close, focus, zoom, read, send-text, send-key
    Pane(cli::pane::PaneCmd),
    /// Print events as they happen, one JSON object per line
    Events(cli::events::EventsCmd),
    /// Read another tool's state: import v1 creates what V1's sessions describe
    Import(cli::import::ImportCmd),
    /// Projects: add, remove, list
    Project(cli::project::ProjectCmd),
    /// Workspaces: create, name, clear-name, clear, delete, list
    Workspace(cli::workspace::WorkspaceCmd),
    /// Register a path as a project and switch to it
    Open(cli::open::OpenCmd),
    /// Agents: list, get, report, focus
    Agent(cli::agent::AgentCmd),
    /// Every agent: kind, place, state, recap
    Peek {
        /// Print the API result rather than the rows
        #[arg(long)]
        json: bool,
    },
    /// This pane's agent: project, workspace, tab and pane
    Whoami,
    /// Send a message to an agent. Messaging arrives in M4
    Send(cli::agent::SendCmd),
    /// Read an agent's last output. Messaging arrives in M4
    Read(cli::agent::ReadCmd),
    /// Wait for an agent to finish. Messaging arrives in M4
    Wait(cli::agent::WaitCmd),
    /// Install the domux hooks for an agent
    Install(cli::install::InstallCmd),
    /// Hold this machine awake: on, off, toggle, status, install
    #[command(name = "stay-awake")]
    StayAwake(cli::stay_awake::StayAwakeCmd),
}

/// Errors go to stderr and leave a status of 1, so a script can tell a failure from an
/// empty answer (principle 12).
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        None => cli::attach::run_bare().await,
        Some(Command::Attach) => cli::attach::run().await,
        Some(Command::Server(c)) => cli::server::run(c).await,
        Some(Command::Config(c)) => cli::config::run(c).await,
        Some(Command::Api(c)) => cli::api::run(c).await,
        Some(Command::Tab(c)) => cli::tab::run(c).await,
        Some(Command::Pane(c)) => cli::pane::run(c).await,
        Some(Command::Events(c)) => cli::events::run(c).await,
        Some(Command::Import(c)) => cli::import::run(c).await,
        Some(Command::Project(c)) => cli::project::run(c).await,
        Some(Command::Workspace(c)) => cli::workspace::run(c).await,
        Some(Command::Open(c)) => cli::open::run(c).await,
        Some(Command::Agent(c)) => cli::agent::run(c).await,
        Some(Command::Peek { json }) => cli::agent::peek(json).await,
        Some(Command::Whoami) => cli::agent::whoami().await,
        Some(Command::Send(c)) => cli::agent::send(c).await,
        Some(Command::Read(c)) => cli::agent::read(c).await,
        Some(Command::Wait(c)) => cli::agent::wait(c).await,
        // The one subcommand that is not an API call: installing hooks needs no server.
        Some(Command::Install(c)) => cli::install::run(c),
        Some(Command::StayAwake(c)) => cli::stay_awake::run(c).await,
    };
    if let Err(e) = result {
        // The whole chain, so the context and the reason under it both reach the reader:
        // "create /tmp/x/sub: Permission denied (os error 13)" rather than the half of it
        // that says what was attempted and not why it failed.
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
