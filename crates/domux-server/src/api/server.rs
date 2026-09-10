//! `server.*`

use super::{ok, Ctx};
use domux_core::api::{Ack, ApiError, ClientInfo, ServerInfo};
use domux_core::proto::PROTOCOL_VERSION;
use serde_json::Value;

pub fn info(ctx: &mut Ctx) -> Result<Value, ApiError> {
    let clients = ctx
        .model
        .clients
        .iter()
        .map(|c| ClientInfo {
            id: c.id.clone(),
            cols: c.size.cols,
            rows: c.size.rows,
            workspace: c.workspace.clone(),
            tab: c.tab.clone(),
        })
        .collect();
    ok(ServerInfo {
        version: domux_core::VERSION.to_string(),
        protocol: PROTOCOL_VERSION,
        socket: ctx.socket_path.clone(),
        state_dir: ctx.state_dir.clone(),
        config_file: ctx.config.path.clone(),
        pid: std::process::id(),
        started_at: ctx.started_at.to_string(),
        // The keymap's, not the file's: what answers keys right now.
        leader: ctx.config.keymap.leader.to_string(),
        config_error: ctx.config.error.as_ref().map(|e| e.to_string()),
        // The hold there is, not the flag saying there should be one.
        stay_awake: ctx.stay_awake.on(),
        clients,
    })
}

pub fn stop(ctx: &mut Ctx) -> Result<Value, ApiError> {
    ctx.stop_requested = true;
    ok(Ack { ok: true })
}
