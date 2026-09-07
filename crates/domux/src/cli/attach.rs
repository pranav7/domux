//! The bare command and `attach`: start the server when needed, then attach.

use super::socket;
use domux_client::{attach, control, AttachOutcome};
use domux_core::names::BIN_NAME;

pub async fn run() -> anyhow::Result<()> {
    let socket = socket();
    if !control::is_live(&socket).await {
        super::server::start().await?;
    }
    // `attach` returns with the terminal already restored, so every line below lands on a
    // terminal the reader can type into again (principle 11).
    match attach(&socket).await? {
        AttachOutcome::Detached(_) => eprintln!("Detached. Run {BIN_NAME} to reattach."),
        AttachOutcome::ServerStopped => eprintln!("The server stopped."),
        AttachOutcome::ConnectionLost => {
            anyhow::bail!("Lost the connection to the server. Run {BIN_NAME} server status.")
        }
        AttachOutcome::Refused(reason) => anyhow::bail!("{reason}"),
    }
    Ok(())
}
