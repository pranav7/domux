//! `list.*`: the cursor inside a box, and what Enter does with the row under it.
//!
//! One set of handlers for every box. The sidebar's Projects box and the switcher's Projects
//! box are the same rows from the same builder, so the cursor that walks them is the same
//! cursor, and M3's two Agents boxes join them here rather than bringing a second set.
//!
//! Nothing here decides what a row looks like or what acting on one does.
//! `render::projects_box` and `render::agents_box` own the rows, and `api::workspace::focus`,
//! `api::agent::focus` and `agent.resume` own the acting; these methods only say which row the
//! keys are on.

use super::{ok, Ctx};
use crate::render::agents_box::{self, RowForm};
use crate::render::list_box::{
    box_lines, content_width, filter_rows, scroll_to_show, text_area, ListRow, OVERLAY_PAD,
    SIDEBAR_PAD,
};
use crate::render::projects_box::{self, Extras};
use domux_core::api::{Ack, AgentTargetParams, ApiError, ClientParams, WorkspaceFocusParams};
use domux_core::ids::{AgentId, ClientId, WorkspaceId};
use domux_core::model::{Focus, Overlay, RegionKind, RowTarget};
use ratatui::layout::Rect;
use serde_json::Value;

/// The box a client's keys are in, as it is drawn now.
struct Visible {
    /// Which box these rows came from, which is what says where the cursor that walks them
    /// is kept: `projects_cursor` for the two Projects boxes, `agents_cursor` for the
    /// Agents box.
    surface: Surface,
    rows: Vec<ListRow>,
    /// Where the fill is: the index of the row the cursor names. `None` when the filter
    /// dropped that row, which is when the box shows no fill at all.
    at: Option<usize>,
    /// The box's inner height, which is what `scroll_to_show` measures against.
    height: u16,
    scroll: u16,
}

/// Which box has this client's keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Switcher,
    /// The Projects box in the sidebar.
    Sidebar,
    /// The Agents box inside the agents overlay (M3).
    AgentsOverlay,
    /// The Agents box in the sidebar, under the Projects box (M3). The same rows as the
    /// overlay's in the narrower two-line form, so the cursor walks what is drawn.
    SidebarAgents,
}

impl Surface {
    /// Whether this box lists workspaces. The switcher's box and the sidebar's are one list
    /// from one builder with one cursor; the Agents box is a different list with a cursor of
    /// its own, so the question is asked once here rather than matched at each use.
    fn is_projects(self) -> bool {
        matches!(self, Surface::Switcher | Surface::Sidebar)
    }
}

/// The box a `list.*` call acts on, or a refusal.
///
/// Every method here is a box operation, so every one of them asks this first: a cursor moved
/// in no box, or a filter opened in no box, is a mode nothing on the screen marks (principle
/// 2). It is also what keeps `visible` and the three renderers reading one list, because the
/// sidebar applies its filter only while its box has the keys - so the surfaces this names are
/// exactly the states in which the filter applies.
fn surface(ctx: &Ctx, client: &ClientId) -> Result<Surface, ApiError> {
    let view = ctx
        .model
        .client(client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    if view.overlay == Some(Overlay::Switcher) {
        return Ok(Surface::Switcher);
    }
    // The overlay and not the focus kind, the same question the switcher's arm asks: an
    // overlay takes every key while it is open, whatever `focus` holds after a help overlay
    // over it closed.
    if view.overlay == Some(Overlay::Agents) {
        return Ok(Surface::AgentsOverlay);
    }
    if view.sidebar_visible() {
        match view.focus {
            Focus::Region(RegionKind::SidebarProjects) => return Ok(Surface::Sidebar),
            Focus::Region(RegionKind::SidebarAgents) => return Ok(Surface::SidebarAgents),
            _ => {}
        }
    }
    Err(ApiError::refused(
        "the keys are not in a box; open the switcher or the agents overlay, or move into the sidebar first",
    ))
}

/// Whether this client's keys are in a **Projects** box.
///
/// `Ctx::workspace_of_view` asks, so that a call carrying no target acts on the row under the
/// cursor exactly when the reader is looking at one, and on the client's own workspace
/// otherwise. It is `surface` and not a second reading of the same state, so the two can
/// never disagree about which box has the keys.
///
/// Neither Agents box is one of these. `n` is bound in `[keys.list]` and the Agents boxes hold
/// those keys too, so a plain "is in a box" here would have `leader N` in the agents overlay
/// rename whatever workspace `projects_cursor` was left pointing at - a row that is not on the
/// screen.
pub(super) fn in_a_projects_box(ctx: &Ctx, client: &ClientId) -> bool {
    surface(ctx, client).is_ok_and(Surface::is_projects)
}

/// The rows this client is looking at: the switcher's when it is open, the agents overlay's
/// when that one is, and the sidebar's Projects or Agents box when the keys are in one of
/// them.
///
/// Every input is the renderer's own - `render::switcher::draw`, `render::agents_overlay::draw`
/// and `render::sidebar::draw` - so the cursor moves over the list on the screen rather than
/// over a second list built to slightly different rules. The width decides where a row is cut
/// and the extras decide how many lines it takes, and both change where `scroll_to_show` puts
/// the view.
///
/// `&mut Ctx` for the agents arm alone: `core::agents_view` assigns a working word to a working
/// agent that has none yet. That is the same call the frame this cursor is moving over makes,
/// and it hands back the word it already assigned on every later call, so a key that moves the
/// cursor cannot give an agent a different word from the one the reader is looking at.
fn visible(ctx: &mut Ctx, client: &ClientId) -> Result<Visible, ApiError> {
    let surface = surface(ctx, client)?;
    let view = ctx
        .model
        .client(client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    let screen = Rect::new(0, 0, view.size.cols, view.size.rows);
    if surface == Surface::SidebarAgents {
        // The sidebar's own Agents box: its rectangle, its narrower form. `render::sidebar`
        // splits the column the same way, so the cursor walks the rows on the screen.
        let (_, area, _) =
            crate::render::sidebar::split_for(ctx.model, ctx.facts, view.size, false);
        let now = ctx.deps.clock.now();
        let agents = crate::core::agents_view(ctx.model, ctx.agents, now);
        let all = agents_box::rows(
            &agents,
            RowForm::Sidebar,
            content_width(area.width, SIDEBAR_PAD),
        );
        let rows = filter_rows(&all, &view.filter);
        let at = projects_box::filled_index(&rows, view.agents_cursor.as_ref().map(|a| a.as_str()));
        return Ok(Visible {
            surface,
            rows,
            at,
            height: text_area(area, SIDEBAR_PAD).height,
            scroll: view.agents_scroll,
        });
    }
    if surface == Surface::AgentsOverlay {
        let width = crate::render::overlay::list_overlay_width(screen);
        let now = ctx.deps.clock.now();
        let agents = crate::core::agents_view(ctx.model, ctx.agents, now);
        let all = agents_box::rows(&agents, RowForm::Overlay, content_width(width, OVERLAY_PAD));
        // The renderer filters the built rows where `projects_box::rows` takes the filter
        // itself, so this arm filters here for the same reason: one list, filtered once, the
        // way the box on the screen was.
        let rows = filter_rows(&all, &view.filter);
        let lines = rows
            .iter()
            .fold(0u16, |sum, r| sum.saturating_add(r.height()));
        let area = crate::render::overlay::list_overlay_area(screen, box_lines(lines, OVERLAY_PAD));
        let at = projects_box::filled_index(&rows, view.agents_cursor.as_ref().map(|a| a.as_str()));
        return Ok(Visible {
            surface,
            rows,
            at,
            height: text_area(area, OVERLAY_PAD).height,
            scroll: view.agents_scroll,
        });
    }
    // The fill is the cursor, and with no cursor it is the workspace this client is in
    // (domain model, section 3.3). `list.*` runs while a box has the keys, which is exactly
    // when both renderers use this same key, so there is one answer and not three.
    let navigator = ctx.config.config.navigator.enabled;
    let key = match navigator {
        true => view.navigator_cursor.as_ref().map(|c| c.as_str()),
        false => view.projects_cursor.as_ref().map(|w| w.as_str()),
    }
    .unwrap_or(view.workspace.as_str())
    .to_string();
    let key = Some(key.as_str());
    // The agents the box nests under their workspaces, or none when the two boxes are on.
    // `core::agents_view` is the same call the frame this cursor moves over makes, so a key
    // cannot give an agent a different word from the one the reader is looking at.
    let now = ctx.deps.clock.now();
    let nested = navigator.then(|| crate::core::agents_view(ctx.model, ctx.agents, now));
    let view = ctx
        .model
        .client(client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    let (rows, height) = if surface == Surface::Switcher {
        let width = crate::render::overlay::list_overlay_width(screen);
        let rows = projects_box::rows(
            ctx.model,
            ctx.facts,
            &view.filter,
            key,
            Extras::switcher(content_width(width, OVERLAY_PAD)),
            nested.as_ref(),
        );
        // The switcher's height follows its rows, the same two passes `switcher::draw` makes:
        // the width does not depend on the rows, and the row count then decides the height.
        let lines = rows
            .rows
            .iter()
            .fold(0u16, |sum, r| sum.saturating_add(r.height()));
        let area = crate::render::overlay::list_overlay_area(screen, box_lines(lines, OVERLAY_PAD));
        (rows, text_area(area, OVERLAY_PAD).height)
    } else {
        let area =
            crate::render::sidebar::projects_area(ctx.model, ctx.facts, view.size, navigator);
        let rows = projects_box::rows(
            ctx.model,
            ctx.facts,
            &view.filter,
            key,
            Extras::compact(content_width(area.width, SIDEBAR_PAD)),
            nested.as_ref(),
        );
        (rows, text_area(area, SIDEBAR_PAD).height)
    };
    Ok(Visible {
        surface,
        at: rows.filled,
        rows: rows.rows,
        height,
        scroll: match navigator {
            true => view.navigator_scroll,
            false => view.projects_scroll,
        },
    })
}

/// Moves the cursor `delta` rows the reader can rest on, and scrolls the box to keep it in
/// view.
fn step(ctx: &mut Ctx, delta: isize) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let v = visible(ctx, &client)?;
    // The cursor rests on the rows that carry a key: a project header and the blank rows
    // between two entries are stepped over, so one press moves one workspace, or one agent,
    // rather than one line (interface spec 12.14).
    let stops: Vec<usize> = v
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.key.is_some())
        .map(|(i, _)| i)
        .collect();
    let Some(first) = stops.first().copied() else {
        // Nothing in the box yet, or a filter that matched nothing. The box says so itself, and
        // a cursor pointed at a row that is not drawn is worse than one that did not move.
        return ok(Ack { ok: true });
    };
    let next = match v.at.and_then(|a| stops.iter().position(|s| *s == a)) {
        Some(here) => stops[(here as isize + delta).clamp(0, stops.len() as isize - 1) as usize],
        // The cursor names a row the filter dropped, so the box is showing no fill. A step
        // starts the cursor at the top rather than moving from a row nobody can see.
        None => first,
    };
    // Every index in `stops` came from a row with a key, so this is always a workspace or an
    // agent, whichever list the box is showing.
    let key = projects_box::key_at(&v.rows, next);
    let scroll = scroll_to_show(&v.rows, Some(next), v.height, v.scroll);
    // Which of the two the key names is a question for the model, not for the shape of the
    // string: the Navigator's rows carry both kinds and an id's prefix is not a contract.
    let target = key.as_deref().and_then(|k| row_target(ctx, k));
    let navigator = ctx.config.config.navigator.enabled;
    if let Some(view) = ctx.model.client_mut(&client) {
        if navigator && v.surface.is_projects() {
            view.navigator_cursor = target;
            view.navigator_scroll = scroll;
        } else if v.surface.is_projects() {
            view.projects_cursor = key.map(WorkspaceId);
            view.projects_scroll = scroll;
        } else {
            view.agents_cursor = key.map(AgentId);
            view.agents_scroll = scroll;
        }
    }
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// What a row key names, asked of the model rather than read off the string.
///
/// The Navigator's rows carry a workspace id or an agent id, and telling them apart by their
/// prefix would make `w_` and `a_` a contract that `Model::next_id` never promised.
fn row_target(ctx: &Ctx, key: &str) -> Option<RowTarget> {
    let workspace = WorkspaceId(key.to_string());
    if ctx.model.workspace(&workspace).is_some() {
        return Some(RowTarget::Workspace(workspace));
    }
    let agent = AgentId(key.to_string());
    ctx.model.agent(&agent).map(|_| RowTarget::Agent(agent))
}

pub fn down(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    step(ctx, 1)
}

pub fn up(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    step(ctx, -1)
}

/// Enter: act on the row under the cursor. In a Projects box that is switching to the
/// workspace; in the Agents box it is switching to the agent, or resuming it when its record
/// has exited (interface spec 6.8).
///
/// The acting is `api::workspace::focus`, `api::agent::focus` and `agent.resume`, and nothing
/// of any of them is repeated here, so the key and the API call reach one handler and cannot
/// drift apart. What this adds is the only thing the key knows and the caller does not: which
/// row the cursor is on.
///
/// It reads the row out of the drawn list rather than out of the cursor directly, so a cursor
/// left on a row the filter has dropped refuses instead of acting on something the box is not
/// showing.
pub fn activate(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let v = visible(ctx, &client)?;
    let Some(key) = v.at.and_then(|i| projects_box::key_at(&v.rows, i)) else {
        return Err(ApiError::not_found(if v.surface.is_projects() {
            "no workspace is under the cursor; move it with the list keys"
        } else {
            "no agent is under the cursor; move it with the list keys"
        }));
    };
    // The Navigator's rows are of two kinds, so what Enter does is decided by what the key
    // names rather than by which box it came from.
    match row_target(ctx, &key) {
        Some(RowTarget::Agent(agent)) => return activate_agent(ctx, &client, agent),
        Some(RowTarget::Workspace(_)) => {}
        // A row whose key names neither, which is a record or a workspace that went between
        // the frame and the key. Refused rather than acted on (principle 4).
        None => {
            return Err(ApiError::not_found(
                "that row is gone; move the cursor with the list keys",
            ))
        }
    }
    if !v.surface.is_projects() {
        return activate_agent(ctx, &client, AgentId(key));
    }
    // `client` describes the request; it is not what steers it. `workspace::focus` reads the
    // acting client from `Ctx::view`, which is already this one - the pressing client for a
    // key and the `client` param for an API call - so the field is filled in truthfully and
    // nothing here depends on it.
    super::workspace::focus(
        ctx,
        WorkspaceFocusParams {
            workspace: key,
            client: Some(client),
        },
    )
}

/// Enter on an agent row: switch to the agent.
///
/// One method and no second implementation of it. Nothing here closes the overlay afterwards:
/// the switch goes through `Model::select_tab`, which puts the client on the tab with no
/// overlay open, so the reader lands in the pane and not behind a box. A switch that refuses
/// leaves the overlay where it was, which is what a reader who pressed Enter on a record that
/// has just gone should see.
///
/// It had a second arm, resume on an exited row, until decision record 0028 removed both the
/// verb and the row.
fn activate_agent(ctx: &mut Ctx, client: &ClientId, agent: AgentId) -> Result<Value, ApiError> {
    super::agent::focus(
        ctx,
        AgentTargetParams {
            agent: Some(agent.to_string()),
            pane: None,
            client: Some(client.clone()),
        },
    )
}

/// `/`: the box filters as you type.
///
/// This only opens the filter. `input::filter_key` owns the typing, because the keys that go
/// into a text field are every key rather than a table of them, and the key that opens it
/// stays configurable like every other (principle 3).
///
/// It asks `surface` like every other method here, and for the same reason: a filter field
/// opened over no box takes every key the reader presses next and nothing on the screen says
/// so.
pub fn filter(ctx: &mut Ctx, _p: ClientParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    surface(ctx, &client)?;
    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    view.filtering = true;
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}
