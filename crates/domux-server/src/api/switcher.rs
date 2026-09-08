//! `switcher.open` and `switcher.close`: a view method pair (roadmap 5.7).

use super::{ok, Ctx};
use domux_core::api::{Ack, ApiError, ClientParams};
use domux_core::model::{Focus, Overlay, RegionKind};
use serde_json::Value;

/// Opens the switcher with the keys in its box and the cursor on the workspace this client
/// is in (interface spec 12.32).
///
/// The filter starts empty every time. `ClientView::filter` outlives an overlay, so this is
/// the decision the field's own comment leaves to whoever opens one: a switcher reopened
/// showing only what the last search matched would hide the workspace the reader came for.
///
/// Nothing in the tree writes `filter` yet - `list.filter` is Task 14 - so that reset is not
/// observable and no test here establishes it. It is written now because this is the method
/// the field's comment points at, and Task 14 should keep it and test it once `/` can type.
///
/// `filtering` is cleared beside it for the same reason and with the same standing: nothing
/// here opens the filter, so deleting the line changes no frame this milestone can draw, and
/// Task 14 owns its test too. Only the open path needs it, because `pop_overlay` already
/// clears `filtering` on the way out.
pub fn open(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let workspace = ctx.model.client(&client).map(|c| c.workspace.clone());
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    if view.overlay == Some(Overlay::Switcher) {
        return ok(Ack { ok: true });
    }
    view.push_overlay(Overlay::Switcher);
    view.projects_cursor = workspace;
    view.filter.clear();
    view.filtering = false;
    view.focus = Focus::Region(RegionKind::Switcher);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Closes it and gives the keys back to what was underneath: the overlay it was opened over
/// (interface spec 12.7), or the pane.
pub fn close(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let focused = ctx.model.client_tab(&client).map(|t| t.focused.clone());
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    if view.overlay != Some(Overlay::Switcher) {
        return ok(Ack { ok: true });
    }
    view.pop_overlay();
    // Never a frame with the keys in a region nothing on the screen marks (principle 2).
    view.focus = match (&view.overlay, focused) {
        (Some(_), _) => Focus::Region(RegionKind::Overlay),
        (None, Some(pane)) => Focus::Pane(pane),
        (None, None) => view.focus.clone(),
    };
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}
