//! `sidebar.toggle`, `show` and `hide`: view methods that also set the remembered state.

use super::{ok, Ctx};
use domux_core::api::{ApiError, ClientParams, Event, SidebarResult};
use domux_core::ids::{ClientId, PaneId};
use domux_core::model::{Focus, RegionKind, SIDEBAR_MIN_COLS};
use serde_json::Value;

/// Shows the sidebar when it is hidden and hides it when it is shown.
///
/// The remembered state decides, not what is on the screen. A sidebar that hid itself
/// because the screen is narrow is still open, so the key that follows hides it rather than
/// showing a sidebar the screen has no room for (interface spec 12.1).
pub fn toggle(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let open = !ctx.model.sidebar_open;
    set(ctx, open)
}

pub fn show(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    set(ctx, true)
}

pub fn hide(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    set(ctx, false)
}

/// Sets the one remembered sidebar state and puts it on every attached client.
///
/// The state is the server's, not the asking client's: `Model::sidebar_open` is a single
/// value, it is what the state file keeps, and it is what a client attaching later starts
/// in (roadmap decision 4). So a client that toggled it does not end up disagreeing with a
/// client that did not, and the model and the views cannot drift apart. What stays each
/// client's own is whether its screen is wide enough to draw the sidebar it has open:
/// `ClientView::sidebar_visible` answers that per client (interface spec 12.1).
///
/// `open` is what the reader asked for; `visible` is what the asking screen has room to
/// draw.
fn set(ctx: &mut Ctx, open: bool) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    // Before anything is set, so a call naming a client that is not attached changes
    // nothing rather than half of it.
    if ctx.model.client(&client).is_none() {
        return Err(ApiError::not_found(format!(
            "client {client} is not attached"
        )));
    }
    ctx.model.set_sidebar_open(open);
    // Each client hands the keys back to its own tab's pane, so the pane the keys land in
    // is the one that client is looking at.
    let handoff: Vec<(ClientId, Option<PaneId>)> = ctx
        .model
        .clients
        .iter()
        .map(|view| {
            (
                view.id.clone(),
                ctx.model
                    .client_tab(&view.id)
                    .map(|tab| tab.focused.clone()),
            )
        })
        .collect();
    for (id, pane) in handoff {
        let asked = id == client;
        let Some(view) = ctx.model.client_mut(&id) else {
            continue;
        };
        view.sidebar_open = open;
        // The reader asking for the sidebar on a screen too narrow to show it on its own
        // gets it: `leader b` shows it at any width (interface spec 12.1). Only for the
        // client that asked, because the auto-hide is each client's own.
        //
        // The `open &&` keeps the field's meaning true at all times - forced is never set
        // while the sidebar is closed - and it cannot be observed today, so no test pins it.
        // `sidebar_forced` is read only through `sidebar_visible`, which ANDs it with
        // `sidebar_open`; the only place a view's `sidebar_open` becomes true is this loop,
        // which re-assigns `sidebar_forced` beside it; and a fresh client gets `false` from
        // `Core` because `Model::clients` is not persisted. So a stale `true` is always
        // overwritten before anything can read it. Kept for the invariant, not for a
        // behaviour, and said here so the next reader does not go looking for the test.
        view.sidebar_forced = open && asked && view.size.cols < SIDEBAR_MIN_COLS;
        if !open && matches!(view.focus, Focus::Region(RegionKind::SidebarProjects)) {
            // Hiding the box the keys were in gives them back to the pane, so no frame is
            // drawn with the keys in a region nothing on the screen marks (principle 2).
            //
            // Unreachable in M2 and therefore untested: nothing sets
            // `Focus::Region(RegionKind::SidebarProjects)` until `api::focus::region`
            // accepts that region, which it refuses today. Written from the rule rather
            // than from a test, so the task that opens the box to focus does not have to
            // rediscover it.
            if let Some(pane) = pane {
                view.focus = Focus::Pane(pane);
            }
        }
    }
    let visible = ctx
        .model
        .client(&client)
        .map(|view| view.sidebar_visible())
        .unwrap_or(false);
    // `SidebarToggled` is a structural event, so publishing it is what writes the new value
    // out to the state file mid-run - and it is the only writer when nothing else changes,
    // which is why `the_sidebar_reaches_the_state_file_while_the_server_is_still_running`
    // attaches a second client at exactly 81 columns. That client makes the toggle resize no
    // pane, so no `PaneResized` writes the file instead. It is not an oddity; it is the test.
    ctx.events.push(Event::SidebarToggled { client, open });
    ctx.view_dirty = true;
    ok(SidebarResult { open, visible })
}
