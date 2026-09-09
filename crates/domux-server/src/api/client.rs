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

/// Opens the keys overlay. Focus moves to the overlay until Esc.
///
/// `push_overlay` and not `overlay = Some(..)`: `?` is a `[keys.list]` key as well as a
/// leader binding, so the switcher is the thing it is most often pressed over, and the
/// switcher has to still be there when Esc closes the help (interface spec 12.7).
/// `input::close_overlay` pops back to it.
///
/// A second call while the help is already open answers yes and changes nothing, the same
/// answer `switcher::open` gives. Without the guard, calling `help` over the API twice would
/// push Help over Help and take two Escs to leave, and the reader would have no way to tell
/// why.
pub fn help(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    if view.overlay == Some(Overlay::Help) {
        return ok(Ack { ok: true });
    }
    view.push_overlay(Overlay::Help);
    // A box keeps its region while the help is over it, because the region is what says which
    // key table the reader is holding and `draw_help` lists that one first. It is not a lie
    // about where the keys are: an open overlay takes every key at step 1 of the routing
    // whatever the focus says, and what is drawn under an overlay keeps its own look already
    // - the switcher under the help draws its focused box and its footer hints.
    if !matches!(view.focus, Focus::Region(k) if k.is_box()) {
        view.focus = Focus::Region(RegionKind::Overlay);
    }
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}
