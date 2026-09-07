//! `domux2 api <method> [json params]` and `domux2 api schema`.

use super::call;
use clap::Args;

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
        println!(
            "{}",
            serde_json::to_string_pretty(&domux_core::api::schema())?
        );
        return Ok(());
    }
    let params: serde_json::Value = match cmd.params {
        Some(p) => serde_json::from_str(&p)
            .map_err(|e| anyhow::anyhow!("params are not valid JSON: {e}"))?,
        None => serde_json::json!({}),
    };
    let result = call(&cmd.method, params).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
