//! `list.*`: the cursor inside a box, and what Enter does with the row under it.
//!
//! One set of handlers for both surfaces. The sidebar's Projects box and the switcher's
//! Projects box are the same rows from the same builder, so the cursor that walks them is
//! the same cursor, and M3's Agents box joins them rather than adding a third.
//!
//! Nothing here decides what a row looks like or what switching to a workspace does.
//! `render::projects_box` owns the rows and `api::workspace::focus` owns the switch; these
//! methods only say which row the keys are on.

use super::{ok, Ctx};
use crate::render::list_box::{scroll_to_show, ListRow};
use crate::render::projects_box::{self, Extras};
use domux_core::api::{Ack, ApiError, ClientParams, WorkspaceFocusParams};
use domux_core::ids::{ClientId, WorkspaceId};
use domux_core::model::{Focus, Overlay, RegionKind};
use ratatui::layout::Rect;
use serde_json::Value;

/// The box a client's keys are in, as it is drawn now.
struct Visible {
    rows: Vec<ListRow>,
    /// Where the fill is: the index of the row the cursor names. `None` when the filter
    /// dropped that row, which is when the box shows no fill at all.
    at: Option<usize>,
    /// The box's inner height, which is what `scroll_to_show` measures against.
    height: u16,
    scroll: u16,
}

/// The rows this client is looking at: the switcher's when it is open, the sidebar's when the
/// keys are in its box.
///
/// It refuses when neither box has the keys, rather than moving a cursor nothing on the
/// screen is drawing (principle 2). That refusal is also what keeps this function and the two
/// renderers in step: the sidebar draws its filter only while its box has the keys, so the
/// two states this answers in are exactly the two states in which the filter applies.
///
/// Every other input is the renderer's own - `render::switcher::draw` for the first arm and
/// `render::sidebar::draw` for the second - so the cursor moves over the list on the screen
/// rather than over a second list built to slightly different rules. The width decides where
/// a row is cut and the extras decide how many lines it takes, and both change where
/// `scroll_to_show` puts the view.
fn visible(ctx: &Ctx, client: &ClientId) -> Result<Visible, ApiError> {
    let view = ctx
        .model
        .client(client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    let in_switcher = view.overlay == Some(Overlay::Switcher);
    let in_sidebar =
        view.sidebar_visible() && matches!(view.focus, Focus::Region(RegionKind::SidebarProjects));
    if !in_switcher && !in_sidebar {
        return Err(ApiError::refused(
            "the keys are not in a box; open the switcher or move into the sidebar first",
        ));
    }
    // The fill is the cursor, and with no cursor it is the workspace this client is in
    // (domain model, section 3.3). `list.*` runs while a box has the keys, which is exactly
    // when both renderers use this same key, so there is one answer and not three.
    let key = view
        .projects_cursor
        .as_ref()
        .map(|w| w.as_str())
        .unwrap_or(view.workspace.as_str());
    let (rows, height) = if in_switcher {
        let screen = Rect::new(0, 0, view.size.cols, view.size.rows);
        let width = crate::render::overlay::list_overlay_width(screen);
        let rows = projects_box::rows(
            ctx.model,
            ctx.facts,
            &view.filter,
            Some(key),
            Extras::switcher(width.saturating_sub(2)),
        );
        // The switcher's height follows its rows, the same two passes `switcher::draw` makes:
        // the width does not depend on the rows, and the row count then decides the height.
        let lines = rows
            .rows
            .iter()
            .fold(0u16, |sum, r| sum.saturating_add(r.height()));
        let area = crate::render::overlay::list_overlay_area(screen, lines);
        (rows, area.height.saturating_sub(2))
    } else {
        let area = crate::render::sidebar::projects_area(view.size);
        let rows = projects_box::rows(
            ctx.model,
            ctx.facts,
            &view.filter,
            Some(key),
            Extras::compact(area.width.saturating_sub(2)),
        );
        (rows, area.height.saturating_sub(2))
    };
    Ok(Visible {
        at: rows.filled,
        rows: rows.rows,
        height,
        scroll: view.projects_scroll,
    })
}

/// Moves the cursor `delta` rows the reader can rest on, and scrolls the box to keep it in
/// view.
fn step(ctx: &mut Ctx, delta: isize) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let v = visible(ctx, &client)?;
    // The cursor rests on workspaces only: a project header and the blank rows between
    // workspaces are stepped over, so one press moves one workspace rather than one line
    // (interface spec 12.14).
    let stops: Vec<usize> = v
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.key.is_some())
        .map(|(i, _)| i)
        .collect();
    let Some(first) = stops.first().copied() else {
        // No project yet, or a filter that matched nothing. The box says so itself, and a
        // cursor pointed at a row that is not drawn is worse than one that did not move.
        return ok(Ack { ok: true });
    };
    let next = match v.at.and_then(|a| stops.iter().position(|s| *s == a)) {
        Some(here) => stops[(here as isize + delta).clamp(0, stops.len() as isize - 1) as usize],
        // The cursor names a row the filter dropped, so the box is showing no fill. A step
        // starts the cursor at the top rather than moving from a row nobody can see.
        None => first,
    };
    // Every index in `stops` came from a row with a key, so this is always a workspace.
    let key = projects_box::key_at(&v.rows, next);
    let scroll = scroll_to_show(&v.rows, Some(next), v.height, v.scroll);
    if let Some(view) = ctx.model.client_mut(&client) {
        view.projects_cursor = key.map(WorkspaceId);
        view.projects_scroll = scroll;
    }
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

pub fn down(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    step(ctx, 1)
}

pub fn up(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    step(ctx, -1)
}

/// Enter: switch to the workspace under the cursor.
///
/// The switching is `api::workspace::focus` and nothing of it is repeated here, so the key
/// and the `workspace.focus` API call reach one handler and cannot drift apart. What this adds
/// is the only thing the key knows and the caller does not: which row the cursor is on.
///
/// It reads the row out of the drawn list rather than out of `projects_cursor` directly, so
/// a cursor left on a row the filter has dropped refuses instead of switching to a workspace
/// the box is not showing.
pub fn activate(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let v = visible(ctx, &client)?;
    let Some(workspace) = v.at.and_then(|i| projects_box::key_at(&v.rows, i)) else {
        return Err(ApiError::not_found(
            "no workspace is under the cursor; move it with the list keys",
        ));
    };
    // `client` describes the request; it is not what steers it. `workspace::focus` reads the
    // acting client from `Ctx::view`, which is already this one - the pressing client for a
    // key and the `client` param for an API call - so the field is filled in truthfully and
    // nothing here depends on it.
    super::workspace::focus(
        ctx,
        WorkspaceFocusParams {
            workspace,
            client: Some(client),
        },
    )
}

/// `/`: the box filters as you type.
///
/// This only opens the filter. `input::filter_key` owns the typing, because the keys that go
/// into a text field are every key rather than a table of them, and the key that opens it
/// stays configurable like every other (principle 3).
pub fn filter(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    view.filtering = true;
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}
