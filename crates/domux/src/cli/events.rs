//! `domux2 events [filter...]`: events.subscribe, one JSON object per line, until Ctrl-C.

use super::{not_running, socket};
use clap::Args;
use domux_client::control;

#[derive(Args)]
pub struct EventsCmd {
    /// Event names or globs such as tab.* ; empty means everything
    pub filter: Vec<String>,
}

pub async fn run(cmd: EventsCmd) -> anyhow::Result<()> {
    let socket = socket();
    if !control::is_live(&socket) {
        return Err(not_running());
    }
    control::subscribe(&socket, cmd.filter, |event| {
        println!("{event}");
        true
    })
    .await
}
