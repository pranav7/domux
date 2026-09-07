//! `focus.*`: view methods. Every one answers with the resulting focus so a caller sees
//! where the keys go now, even when nothing moved.

use super::{ok, Ctx};
use domux_core::api::{ApiError, ClientParams, FocusRegionParams, FocusResult};
use domux_core::model::layout::{neighbour_by_geometry, solve};
use domux_core::model::{Direction, Focus, RegionKind};
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
    }
    result(ctx)
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

/// M1 has one region: the overlay, and only while one is open. The switcher and sidebar
/// kinds arrive with M2, the agents overlay with M3.
pub fn region(ctx: &mut Ctx, p: FocusRegionParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
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
        other => {
            return Err(ApiError::unavailable(format!(
                "region {other:?} arrives with the switcher and sidebar in M2 and the agents overlay in M3"
            )))
        }
    }
    ctx.view_dirty = true;
    result(ctx)
}

/// Back to the pane: closes an open overlay and clears a pending chord.
pub fn pane(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let focused = ctx.model.client_tab(&client).map(|t| t.focused.clone());
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    view.overlay = None;
    view.chord = None;
    if let Some(p) = focused {
        view.focus = Focus::Pane(p);
    }
    ctx.view_dirty = true;
    result(ctx)
}
