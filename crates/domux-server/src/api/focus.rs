//! `focus.*`: view methods. Every one answers with the resulting focus so a caller sees
//! where the keys go now, even when nothing moved.

use super::{ok, Ctx};
use domux_core::api::{ApiError, ClientParams, FocusRegionParams, FocusResult};
use domux_core::ids::ClientId;
use domux_core::model::layout::{neighbour_by_geometry, solve};
use domux_core::model::{ClientView, Direction, Focus, Overlay, RegionKind};
use serde_json::Value;

fn result(ctx: &Ctx) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let focus = ctx
        .model
        .client(&client)
        .map(|c| c.focus.clone())
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    ok(FocusResult { focus })
}

pub fn step(ctx: &mut Ctx, _p: ClientParams, dir: Direction) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    // A region answers first. The keys are in a box, so the pane neighbours are not what the
    // reader is asking about, and stepping over them would move the focused pane of a tab
    // nobody is typing into.
    if let Some(region) = ctx.model.client(&client).and_then(|v| match v.focus {
        Focus::Region(kind) => Some(kind),
        Focus::Pane(_) => None,
    }) {
        return step_from_region(ctx, &client, region, dir);
    }
    let tab = ctx
        .model
        .client_tab(&client)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("client {client} has no tab")))?;
    let area = ctx.smallest_area(&tab.id);
    let rects = solve(&tab.layout, area, tab.zoomed.as_ref());
    if let Some(next) = neighbour_by_geometry(&rects, &tab.focused, dir) {
        let events = ctx.model.focus_pane(&next)?;
        ctx.events.extend(events);
        ctx.view_dirty = true;
    } else if dir == Direction::Left
        && ctx
            .model
            .client(&client)
            .is_some_and(|v| v.sidebar_visible())
    {
        // `C-h` from a pane against the workpanel's left edge enters the sidebar's Projects
        // box. Geometry first, so a pane that has a left neighbour still moves to it: the
        // sidebar is what lies past the edge, not what lies past the pane (interface spec
        // 12.29). M2 has one box in the sidebar, so nothing here has to choose between two.
        let workspace = ctx.model.client(&client).map(|v| v.workspace.clone());
        if let Some(view) = ctx.model.client_mut(&client) {
            enter_projects_box(view, workspace);
        }
        ctx.view_dirty = true;
    }
    result(ctx)
}

/// `focus.*` from a region.
///
/// M2 has one box outside an overlay, the sidebar's Projects box. It is the leftmost thing
/// on the screen and it fills the sidebar's column, so only `focus.right` has anywhere to
/// go and it hands the keys back to the pane; M3 adds the Agents box under it and gives
/// `focus.down` and `focus.up` somewhere to land. A region inside an overlay moves nowhere
/// at all: the overlay owns its keys until it closes, and a frame with the keys on a pane
/// under an open overlay marks the wrong thing (principle 2).
///
/// Answering with the focus it found rather than refusing is M1's rule for every `focus.*`
/// call: a caller sees where the keys are now, even when nothing moved.
fn step_from_region(
    ctx: &mut Ctx,
    client: &ClientId,
    region: RegionKind,
    dir: Direction,
) -> Result<Value, ApiError> {
    if region == RegionKind::SidebarProjects && dir == Direction::Right {
        return pane(
            ctx,
            ClientParams {
                client: Some(client.clone()),
            },
        );
    }
    result(ctx)
}

/// Puts the keys in the Projects box with the cursor on the row the fill was already on,
/// which is the workspace this client is in (domain model, section 3.3). `focus.left` and
/// `focus.region sidebar_projects` are two ways to the same place, so they enter it once.
///
/// The filter starts empty, the same decision `api::switcher::open` makes for the same
/// reason: a box reopened showing only what the last search matched would hide the workspace
/// the reader came for. Together with `render::sidebar::draw` reading `filter` only while the
/// box has the keys, this is the whole rule - the filter lives exactly as long as the box's
/// hold on the keys - and it is why nothing else has to remember to clear it on the way out.
fn enter_projects_box(view: &mut ClientView, workspace: Option<domux_core::ids::WorkspaceId>) {
    view.focus = Focus::Region(RegionKind::SidebarProjects);
    view.projects_cursor = workspace;
    view.filter.clear();
    view.filtering = false;
}

pub fn last(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let tab = ctx
        .model
        .client_tab(&client)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("client {client} has no tab")))?;
    if let Some(prev) = tab.last_focused.filter(|p| tab.layout.contains(p)) {
        let events = ctx.model.focus_pane(&prev)?;
        ctx.events.extend(events);
        ctx.view_dirty = true;
    }
    result(ctx)
}

/// The regions a client can put its keys in: the overlay while one is open, the switcher's
/// box while the switcher is open, and the sidebar's Projects box while the sidebar shows.
/// M3 adds the two agents kinds.
///
/// Each one refuses when the thing it names is not on the screen, because a frame with the
/// keys in a region nothing marks tells the reader nothing (principle 2). The word is
/// `refused` and not `unavailable`: the region exists, the screen is not showing it, and the
/// message says what to do about that.
pub fn region(ctx: &mut Ctx, p: FocusRegionParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let workspace = ctx.model.client(&client).map(|v| v.workspace.clone());
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    match p.region {
        RegionKind::Overlay if view.overlay.is_some() => {
            view.focus = Focus::Region(RegionKind::Overlay)
        }
        RegionKind::Overlay => {
            return Err(ApiError::refused(
                "no overlay is open; open one with the help key or the tab prompt",
            ))
        }
        RegionKind::Switcher if view.overlay == Some(Overlay::Switcher) => {
            view.focus = Focus::Region(RegionKind::Switcher)
        }
        RegionKind::Switcher => {
            return Err(ApiError::refused(
                "the switcher is not open; open it with switcher.open",
            ))
        }
        RegionKind::SidebarProjects if view.sidebar_visible() => {
            enter_projects_box(view, workspace)
        }
        RegionKind::SidebarProjects => {
            return Err(ApiError::refused(
                "the sidebar is not showing; show it with sidebar.show",
            ))
        }
        other => {
            return Err(ApiError::unavailable(format!(
                "region {other:?} arrives with the agents overlay in M3"
            )))
        }
    }
    ctx.view_dirty = true;
    result(ctx)
}

/// Back to the pane: closes the overlay that has the keys and clears a pending chord.
///
/// "The pane" is where the keys land when nothing else is open. An overlay opened over
/// another leaves that one open and hands the keys to it instead, so a reader who pressed
/// Esc once goes back one step rather than losing both (interface spec 12.7).
pub fn pane(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let focused = ctx.model.client_tab(&client).map(|t| t.focused.clone());
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    // One overlay, not the whole stack. `Esc` in the switcher's box is this method - the
    // `[keys.list]` table binds it to `focus.pane` - and a switcher opened over another
    // overlay gives the keys back to that one, not to the pane (interface spec 12.7).
    // `pop_overlay` is what uncovers it; setting `overlay` to `None` instead would strand
    // `overlay_under` where nothing draws it and nothing closes it.
    view.pop_overlay();
    view.chord = None;
    view.focus = view.focus_after_pop(focused);
    ctx.view_dirty = true;
    result(ctx)
}
