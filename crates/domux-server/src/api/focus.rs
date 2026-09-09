//! `focus.*`: view methods. Every one answers with the resulting focus so a caller sees
//! where the keys go now, even when nothing moved.

use super::{ok, Ctx};
use crate::render::sidebar;
use domux_core::api::{ApiError, ClientParams, FocusRegionParams, FocusResult};
use domux_core::ids::ClientId;
use domux_core::model::layout::{neighbour_by_geometry, solve};
use domux_core::model::{Direction, Focus, Overlay, RegionKind};
use ratatui::layout::Rect;
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
        // `C-h` from a pane against the workpanel's left edge enters the sidebar. Geometry
        // first, so a pane that has a left neighbour still moves to it: the sidebar is what
        // lies past the edge, not what lies past the pane. Which of the two boxes it enters
        // is the one whose rows overlap this pane's most (interface spec 12.29).
        let size = ctx.model.client(&client).map(|v| v.size);
        let pane_rect = rects
            .iter()
            .find(|(id, _)| id == &tab.focused)
            .map(|(_, rect)| *rect);
        let region = match (size, pane_rect) {
            (Some(size), Some(pane)) => {
                let (projects, agents, _) = sidebar::split_column(sidebar::sidebar_area(size));
                sidebar::region_for_rows(as_ratatui(pane), projects, agents)
            }
            // No rectangle to measure, which is a tab with no panes. The upper box is where
            // `C-h` went before there were two (interface spec 12.29's tie).
            _ => RegionKind::SidebarProjects,
        };
        enter_sidebar_box(ctx, &client, region);
        ctx.view_dirty = true;
    }
    result(ctx)
}

/// A layout rectangle as the renderer's geometry spells it. The two types hold the same four
/// numbers; `domux_core::model::Rect` is the one the layout solver and the state file use and
/// `ratatui::layout::Rect` is the one every box is drawn into.
fn as_ratatui(rect: domux_core::model::Rect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

/// `focus.*` from a region.
///
/// The sidebar holds the two boxes outside an overlay, one above the other in the leftmost
/// column of the screen. `focus.right` hands the keys back to the pane from either; `down`
/// crosses from Projects to Agents and `up` back, and from the box at that end they change
/// nothing rather than wrapping round. A region inside an overlay moves nowhere at all: the
/// overlay owns its keys until it closes, and a frame with the keys on a pane under an open
/// overlay marks the wrong thing (principle 2).
///
/// Answering with the focus it found rather than refusing is M1's rule for every `focus.*`
/// call: a caller sees where the keys are now, even when nothing moved.
fn step_from_region(
    ctx: &mut Ctx,
    client: &ClientId,
    region: RegionKind,
    dir: Direction,
) -> Result<Value, ApiError> {
    let sidebar = matches!(
        region,
        RegionKind::SidebarProjects | RegionKind::SidebarAgents
    );
    if sidebar && dir == Direction::Right {
        return pane(
            ctx,
            ClientParams {
                client: Some(client.clone()),
            },
        );
    }
    let crossed = match (region, dir) {
        (RegionKind::SidebarProjects, Direction::Down) => Some(RegionKind::SidebarAgents),
        (RegionKind::SidebarAgents, Direction::Up) => Some(RegionKind::SidebarProjects),
        _ => None,
    };
    if let Some(region) = crossed {
        enter_sidebar_box(ctx, client, region);
        ctx.view_dirty = true;
    }
    result(ctx)
}

/// `Tab`: the other box in the sidebar (interface spec 12.27).
///
/// An overlay holds one box, so there is nothing in it to cross to and the keys stay where
/// they are. Answering with the focus rather than refusing is the rule every `focus.*` call
/// follows.
pub fn next_region(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let across = match ctx.model.client(&client).map(|v| &v.focus) {
        Some(Focus::Region(RegionKind::SidebarProjects)) => Some(RegionKind::SidebarAgents),
        Some(Focus::Region(RegionKind::SidebarAgents)) => Some(RegionKind::SidebarProjects),
        _ => None,
    };
    if let Some(region) = across {
        enter_sidebar_box(ctx, &client, region);
        ctx.view_dirty = true;
    }
    result(ctx)
}

/// Puts the keys in one of the sidebar's two boxes, with a cursor on a row it is showing.
///
/// Every way in comes through here - `focus.left`, `focus.next_region`, `focus.up`,
/// `focus.down` and `focus.region` - so the two boxes are entered one way and a reader
/// cannot land in one with no fill on it.
fn enter_sidebar_box(ctx: &mut Ctx, client: &ClientId, region: RegionKind) {
    let workspace = ctx.model.client(client).map(|v| v.workspace.clone());
    let first_agent = ctx.model.sorted_agents().first().map(|a| a.id.clone());
    let known = |id: &domux_core::ids::AgentId| ctx.model.agent(id).is_some();
    let cursor = match ctx
        .model
        .client(client)
        .and_then(|v| v.agents_cursor.clone())
    {
        Some(held) if known(&held) => Some(held),
        // No cursor yet, or one naming a record that is gone: the first row (interface spec
        // 12.32). A cursor on a row the box is not showing marks nothing (principle 2).
        _ => first_agent,
    };
    let Some(view) = ctx.model.client_mut(client) else {
        return;
    };
    match region {
        RegionKind::SidebarAgents => {
            view.focus = Focus::Region(RegionKind::SidebarAgents);
            view.agents_cursor = cursor;
        }
        // The Projects box, whose cursor starts on the row the fill was already on: the
        // workspace this client is in (domain model, section 3.3).
        _ => {
            view.focus = Focus::Region(RegionKind::SidebarProjects);
            view.projects_cursor = workspace;
        }
    }
    // The filter starts empty, the same decision `api::switcher::open` makes for the same
    // reason: a box reopened showing only what the last search matched would hide the row the
    // reader came for. Together with `render::sidebar::draw` reading `filter` only while a
    // box has the keys, this is the whole rule - the filter lives exactly as long as a box's
    // hold on the keys - and it is why nothing else has to remember to clear it on the way
    // out. One field for both boxes, so crossing between them clears it too.
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
/// box while the switcher is open, the agents overlay's box while that is open, and the
/// sidebar's two boxes while the sidebar shows.
///
/// Each one refuses when the thing it names is not on the screen, because a frame with the
/// keys in a region nothing marks tells the reader nothing (principle 2). The word is
/// `refused` and not `unavailable`: the region exists, the screen is not showing it, and the
/// message says what to do about that.
///
/// The match is exhaustive over `RegionKind`, so a region added later is a compile error here
/// rather than a call that answers with a milestone's name long after that milestone shipped.
pub fn region(ctx: &mut Ctx, p: FocusRegionParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let view = ctx
        .model
        .client(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    // Whether the thing this region names is on the screen, asked once. Both matches are
    // exhaustive over `RegionKind`, so a region added later is a compile error here rather
    // than a call that lands the keys somewhere nothing marks.
    let showing = match p.region {
        RegionKind::Overlay => view.overlay.is_some(),
        RegionKind::Switcher => view.overlay == Some(Overlay::Switcher),
        RegionKind::AgentsOverlay => view.overlay == Some(Overlay::Agents),
        RegionKind::SidebarProjects | RegionKind::SidebarAgents => view.sidebar_visible(),
    };
    if !showing {
        return Err(ApiError::refused(match p.region {
            RegionKind::Overlay => {
                "no overlay is open; open one with the help key or the tab prompt"
            }
            RegionKind::Switcher => "the switcher is not open; open it with switcher.open",
            RegionKind::AgentsOverlay => "the agents overlay is not open; open it with agents.open",
            RegionKind::SidebarProjects | RegionKind::SidebarAgents => {
                "the sidebar is not showing; show it with sidebar.show"
            }
        }));
    }
    match p.region {
        // The sidebar's boxes carry a cursor and a filter with them, which is more than a
        // focus assignment, so they go through the one function every way in uses.
        RegionKind::SidebarProjects | RegionKind::SidebarAgents => {
            enter_sidebar_box(ctx, &client, p.region)
        }
        kind => {
            if let Some(view) = ctx.model.client_mut(&client) {
                view.focus = Focus::Region(kind);
            }
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
    // The pane, not the box: this method is the request to leave (interface spec 5.4).
    let pane_focus = view.focus_on_pane(focused);
    view.focus = view.focus_after_pop(pane_focus);
    ctx.view_dirty = true;
    result(ctx)
}
