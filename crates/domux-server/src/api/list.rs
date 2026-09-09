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
use crate::agents::context::place_of;
use crate::render::agents_box::{self, RowForm};
use crate::render::list_box::{filter_rows, scroll_to_show, ListRow};
use crate::render::projects_box::{self, Extras};
use domux_core::api::{
    Ack, AgentResumeParams, AgentTargetParams, ApiError, ClientParams, Method, WorkspaceFocusParams,
};
use domux_core::ids::{AgentId, ClientId, WorkspaceId};
use domux_core::model::agent::AgentState;
use domux_core::model::{Focus, Model, Overlay, RegionKind};
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
            crate::render::sidebar::split_column(crate::render::sidebar::sidebar_area(view.size));
        let now = ctx.deps.clock.now();
        let agents = crate::core::agents_view(ctx.model, ctx.agents, &ctx.config.keymap, now);
        let all = agents_box::rows(&agents, RowForm::Sidebar, area.width.saturating_sub(2));
        let rows = filter_rows(&all, &view.filter);
        let at = projects_box::filled_index(&rows, view.agents_cursor.as_ref().map(|a| a.as_str()));
        return Ok(Visible {
            surface,
            rows,
            at,
            height: area.height.saturating_sub(2),
            scroll: view.agents_scroll,
        });
    }
    if surface == Surface::AgentsOverlay {
        let width = crate::render::overlay::list_overlay_width(screen);
        let now = ctx.deps.clock.now();
        let agents = crate::core::agents_view(ctx.model, ctx.agents, &ctx.config.keymap, now);
        let all = agents_box::rows(&agents, RowForm::Overlay, width.saturating_sub(2));
        // The renderer filters the built rows where `projects_box::rows` takes the filter
        // itself, so this arm filters here for the same reason: one list, filtered once, the
        // way the box on the screen was.
        let rows = filter_rows(&all, &view.filter);
        let lines = rows
            .iter()
            .fold(0u16, |sum, r| sum.saturating_add(r.height()));
        let area = crate::render::overlay::list_overlay_area(screen, lines);
        let at = projects_box::filled_index(&rows, view.agents_cursor.as_ref().map(|a| a.as_str()));
        return Ok(Visible {
            surface,
            rows,
            at,
            height: area.height.saturating_sub(2),
            scroll: view.agents_scroll,
        });
    }
    // The fill is the cursor, and with no cursor it is the workspace this client is in
    // (domain model, section 3.3). `list.*` runs while a box has the keys, which is exactly
    // when both renderers use this same key, so there is one answer and not three.
    let key = view
        .projects_cursor
        .as_ref()
        .map(|w| w.as_str())
        .unwrap_or(view.workspace.as_str());
    let (rows, height) = if surface == Surface::Switcher {
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
        surface,
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
    if let Some(view) = ctx.model.client_mut(&client) {
        if v.surface.is_projects() {
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

/// Enter on an agent row: switch to the agent, or resume it when its record has exited
/// (interface spec 6.8).
///
/// Two methods, and no third implementation of either.
///
/// A live record is `agent.focus`, and nothing here closes the overlay afterwards: the switch
/// goes through `Model::select_tab`, which puts the client on the tab with no overlay open, so
/// the reader lands in the pane and not behind a box (plan assumption 28). A second close
/// would be a line that never runs. A switch that refuses leaves the overlay where it was,
/// which is what a reader who pressed Enter on a record that has just gone should see.
///
/// An exited record is `agent.resume`, reached through `dispatch` rather than by calling the
/// handler, so the row reaches whatever `agent.resume` is with nothing to edit here in between.
/// The overlay stays open on that path (plan assumption 28), because resume is not a navigation:
/// the reader is still looking at the list, and the row they pressed on is still exited until the
/// resumed session reports its first hook.
///
/// So the overlay's footer is where the result goes, and it is the one thing this adds to either
/// method. Nothing else on the screen changes: a resume types a line into a pane the reader
/// cannot see from here, and a refusal changes nothing at all, so without a line in the footer
/// Enter on an exited row would look like a key that did nothing (principle 8). The pill is set
/// here rather than in `agent::resume`, because a caller with a command line has its answer in
/// the reply and does not need one written on somebody's screen; the same reason `input`'s name
/// box sets the pill for `workspace.rename` rather than the handler doing it.
fn activate_agent(ctx: &mut Ctx, client: &ClientId, agent: AgentId) -> Result<Value, ApiError> {
    let exited = ctx
        .model
        .agent(&agent)
        .is_some_and(|a| a.state == AgentState::Exited);
    if exited {
        let done = super::dispatch(
            Method::AgentResume(AgentResumeParams {
                agent: Some(agent.to_string()),
                client: Some(client.clone()),
            }),
            ctx,
        );
        match &done {
            // The kind and the place are read back out of the record rather than out of the
            // result, which carries the pane and the command: the footer says which agent came
            // back and where it is, and a shell line is not that.
            Ok(_) => {
                let model: &Model = ctx.model;
                // The record is there: `agent.resume` just read it, and nothing removes one.
                if let Some(said) = model
                    .agent(&agent)
                    .map(|a| format!("Resumed {} in {}", a.kind, place_of(model, a)))
                {
                    ctx.set_pill(said, true);
                }
            }
            Err(e) => ctx.set_pill(e.message.clone(), false),
        }
        return done;
    }
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
