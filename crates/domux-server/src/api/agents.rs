//! `agents.open` and `agents.close`: the overlay that lists the records, a view method pair
//! (roadmap 5.7). `agent.*` is the records namespace and lives next door in `api::agent`.

use super::{ok, Ctx};
use domux_core::api::{Ack, ApiError, ClientParams};
use domux_core::model::{Focus, Overlay, RegionKind};
use serde_json::Value;

/// Opens the overlay with the keys in the Agents box and the cursor on the first row
/// (interface spec 12.32).
///
/// The first row is the first record in `Model::sorted_agents`, which is the order the box
/// draws, so the cursor starts on the agent the reader is looking at rather than on whichever
/// record happens to be first in the model.
///
/// The filter starts empty every time, and `agents_scroll` with it, for the reason
/// `switcher::open` gives: `ClientView::filter` outlives an overlay, and a box reopened
/// showing only what the last search matched would hide the agent the reader came for.
///
/// `push_overlay` and not an assignment, so an overlay this one was opened over comes back
/// when it closes (interface spec 12.7) rather than being stranded in `overlay_under` where
/// nothing draws it and nothing closes it.
///
/// **With the Navigator on it does nothing, silently** (decision record 0028). The agents
/// overlay is gone: every record is in the one list the switcher opens, under the workspace it
/// runs in. It answers `ok` rather than refusing, because a key that only ever prints "this is
/// off now" is a key with nothing to say, and the reader who pressed it finds the same records
/// one key away. It is the `[navigator]` key that keeps this handler, and both go together.
pub fn open(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    if ctx.config.config.navigator.enabled {
        return ok(Ack { ok: true });
    }
    let first = ctx.model.sorted_agents().first().map(|a| a.id.clone());
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    if view.overlay == Some(Overlay::Agents) {
        return ok(Ack { ok: true });
    }
    view.push_overlay(Overlay::Agents);
    view.agents_cursor = first;
    view.agents_scroll = 0;
    view.filter.clear();
    view.filtering = false;
    view.focus = Focus::Region(RegionKind::AgentsOverlay);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Closes it and gives the keys back to what was underneath: the overlay it was opened over
/// (interface spec 12.7), or the tab's focused pane (plan assumption 27).
///
/// `api::focus::pane` is that operation and every step of it is there, not repeated here, so
/// `Esc` in the box - which `[keys.list]` binds to `focus.pane` - and this call are one
/// implementation. What this adds is the guard: `agents.close` names the agents overlay, so it
/// closes that one and not whatever else the client has open.
pub fn close(ctx: &mut Ctx, p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let open = ctx
        .model
        .client(&client)
        .is_some_and(|v| v.overlay == Some(Overlay::Agents));
    if !open {
        return ok(Ack { ok: true });
    }
    super::focus::pane(ctx, p)?;
    ok(Ack { ok: true })
}
