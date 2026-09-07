//! `client.*` and `help`: view methods.

use super::{ok, Ctx};
use domux_core::api::{Ack, ApiError, ClientParams};
use domux_core::model::{Focus, Overlay, RegionKind};
use serde_json::Value;

/// Detaches the view. The core sends `Detached` and closes the connection.
pub fn detach(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    ctx.detach_clients.push(client);
    ok(Ack { ok: true })
}

/// Opens the keys overlay (Task 18 draws it). Focus moves to the overlay until Esc.
pub fn help(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    view.overlay = Some(Overlay::Help);
    view.focus = Focus::Region(RegionKind::Overlay);
    ok(Ack { ok: true })
}
