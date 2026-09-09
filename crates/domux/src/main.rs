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
    };
    if let Err(e) = result {
        // The whole chain, so the context and the reason under it both reach the reader:
        // "create /tmp/x/sub: Permission denied (os error 13)" rather than the half of it
        // that says what was attempted and not why it failed.
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
