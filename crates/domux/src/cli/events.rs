//! `events [filter...]`: events.subscribe, one JSON object per line, until Ctrl-C.

use super::{not_running, socket, write_line, Wrote};
use clap::Args;
use domux_client::control;

#[derive(Args)]
pub struct EventsCmd {
    /// Event names or globs such as tab.* ; empty means everything
    pub filter: Vec<String>,
}

pub async fn run(cmd: EventsCmd) -> anyhow::Result<()> {
    let socket = socket();
    if !control::is_live(&socket).await {
        return Err(not_running());
    }
    // `events | head` is the normal way to read a stream, and it leaves the writer with a
    // pipe nobody is reading. That ends the subscription with a status of 0; a write that
    // failed for any other reason is kept and reported after the stream ends.
    let mut out = std::io::stdout().lock();
    let mut failure = None;
    control::subscribe(&socket, cmd.filter, |event| {
        match write_line(&mut out, &event.to_string()) {
            Ok(Wrote::Line) => true,
            Ok(Wrote::ReaderGone) => false,
            Err(e) => {
                failure = Some(e);
                false
            }
        }
    })
    .await?;
    match failure {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
