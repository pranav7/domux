//! `server.*`

use super::{ok, Ctx};
use domux_core::api::{Ack, ApiError, ClientInfo, ServerInfo, ServerUpgradeParams};
use domux_core::names::BIN_NAME;
use domux_core::proto::PROTOCOL_VERSION;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;

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
        upgraded_at: ctx.upgraded_at.map(str::to_string),
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

/// Replaces this server with `binary`, keeping every pane (decision 0046). Everything that
/// would stop the upgrade before it had changed anything is refused here, and the core does
/// the rest, which is also what answers the caller.
pub fn upgrade(ctx: &mut Ctx, p: ServerUpgradeParams) -> Result<Value, ApiError> {
    if ctx.upgrading {
        return Err(ApiError::busy("the server is already upgrading"));
    }
    if p.handoff != crate::upgrade::HANDOFF_FORMAT {
        return Err(ApiError::conflict(format!(
            "this server hands over in format {} and {} reads format {}, so it cannot upgrade in place; run {BIN_NAME} server restart, which ends every pane",
            crate::upgrade::HANDOFF_FORMAT,
            p.binary.display(),
            p.handoff
        )));
    }
    if !p.binary.is_absolute() {
        return Err(ApiError::invalid_params(format!(
            "binary must be an absolute path, got {}",
            p.binary.display()
        )));
    }
    let runnable = std::fs::metadata(&p.binary)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false);
    if !runnable {
        return Err(ApiError::invalid_params(format!(
            "{} is not an executable file",
            p.binary.display()
        )));
    }
    ctx.upgrade = Some(p.binary);
    ctx.defer_reply = true;
    ok(Ack { ok: true })
}
