//! `switcher.open` and `switcher.close`: a view method pair (roadmap 5.7).

use super::{ok, Ctx};
use domux_core::api::{Ack, ApiError, ClientParams};
use domux_core::model::{Focus, Overlay, RegionKind, RowTarget};
use serde_json::Value;

/// Opens the switcher with the keys in its box, the cursor on the workspace this client is
/// in (interface spec 12.32), and the filter field open, so the first letter typed narrows
/// the list (decision record 0059).
///
/// The filter starts empty every time. `ClientView::filter` outlives an overlay, and a
/// switcher reopened showing only what the last search matched would hide the workspace the
/// reader came for.
///
/// The field is open from the start because the switcher is a quick switch: open it, type
/// part of a name, press Enter. The keys that act on a row are a step of their own, behind
/// Tab, where the letters are the `[keys.list]` table again and not text.
pub fn open(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let navigator = ctx.config.config.navigator.enabled;
    let workspace = ctx.model.client(&client).map(|c| c.workspace.clone());
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    if view.overlay == Some(Overlay::Switcher) {
        return ok(Ack { ok: true });
    }
    view.push_overlay(Overlay::Switcher);
    // The same row either way: the workspace this client is in. The Navigator keeps its own
    // cursor because its rows are of two kinds (decision record 0030).
    match navigator {
        true => {
            view.navigator_cursor = workspace.map(RowTarget::Workspace);
            view.navigator_scroll = 0;
        }
        false => view.projects_cursor = workspace,
    }
    view.filter.clear();
    view.filtering = true;
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
    let pane_focus = view.focus_on_pane(focused);
    view.focus = view.focus_after_pop(pane_focus);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}
