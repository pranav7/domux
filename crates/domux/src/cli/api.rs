//! `api <method> [json params]` and `api schema`.

use super::{call, print_line};
use clap::Args;
use domux_core::names::BIN_NAME;

#[derive(Args)]
pub struct ApiCmd {
    /// A method name such as pane.split, or "schema"
    pub method: String,
    /// Params as a JSON object; default {}
    pub params: Option<String>,
}

pub async fn run(cmd: ApiCmd) -> anyhow::Result<()> {
    // The schema describes this build, not a running server, so it answers with nothing
    // listening.
    if cmd.method == "schema" {
        // The schema is this build's own answer, so params cannot mean anything to it.
        // Printing the schema and saying nothing would read as though they had been used.
        if cmd.params.is_some() {
            anyhow::bail!("{BIN_NAME} api schema takes no params. Run it with no argument.");
        }
        print_line(&serde_json::to_string_pretty(&domux_core::api::schema())?)?;
        return Ok(());
    }
    let params: serde_json::Value = match cmd.params {
        Some(p) => serde_json::from_str(&p)
            .map_err(|e| anyhow::anyhow!("params are not valid JSON: {e}"))?,
        None => serde_json::json!({}),
    };
    let result = call(&cmd.method, params).await?;
    print_line(&serde_json::to_string_pretty(&result)?)
}
