//! `tab.*`

use super::{ok, Ctx};
use domux_core::api::{
    Ack, ApiError, TabCreateParams, TabInfo, TabListParams, TabRenameParams, TabSelectParams,
    TabTargetParams,
};
use domux_core::ids::{TabId, WorkspaceId};
use domux_core::model::{Focus, Overlay, PromptKind, RegionKind, TextInput};
use serde_json::Value;

fn info(ctx: &Ctx, ws: &WorkspaceId, tab: &TabId) -> Result<TabInfo, ApiError> {
    let w = ctx
        .model
        .workspace(ws)
        .ok_or_else(|| ApiError::not_found(format!("workspace {ws} does not exist")))?;
    let (i, t) = w
        .tabs
        .iter()
        .enumerate()
        .find(|(_, t)| &t.id == tab)
        .ok_or_else(|| {
            ApiError::not_found(format!("tab {tab} does not exist; run domux2 api tab.list"))
        })?;
    Ok(TabInfo {
        id: t.id.clone(),
        index: i + 1,
        name: t.name.clone(),
        panes: t.layout.pane_ids(),
        focused: t.focused.clone(),
        zoomed: t.zoomed.clone(),
    })
}

fn workspace_param(ctx: &Ctx, ws: Option<&str>) -> Result<WorkspaceId, ApiError> {
    match ws {
        Some(s) => {
            let id: WorkspaceId = s.parse().map_err(|_| {
                ApiError::invalid_params(format!(
                    "{s:?} is not a workspace id; workspace ids look like w_c3a1"
                ))
            })?;
            ctx.model
                .workspace(&id)
                .map(|w| w.id.clone())
                .ok_or_else(|| ApiError::not_found(format!("workspace {s} does not exist")))
        }
        None => {
            let client = ctx.view()?;
            ctx.model
                .client(&client)
                .map(|c| c.workspace.clone())
                .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))
        }
    }
}

pub fn list(ctx: &mut Ctx, p: TabListParams) -> Result<Value, ApiError> {
    let ws = workspace_param(ctx, p.workspace.as_deref())?;
    let ids: Vec<TabId> = ctx
        .model
        .workspace(&ws)
        .map(|w| w.tabs.iter().map(|t| t.id.clone()).collect())
        .unwrap_or_default();
    let infos: Result<Vec<TabInfo>, ApiError> = ids.iter().map(|t| info(ctx, &ws, t)).collect();
    ok(infos?)
}

/// A new tab with one shell in the workspace path (or `cwd`), selected in the calling view.
pub fn create(ctx: &mut Ctx, p: TabCreateParams) -> Result<Value, ApiError> {
    let ws = workspace_param(ctx, p.workspace.as_deref())?;
    let cwd = p
        .cwd
        .clone()
        .or_else(|| ctx.model.workspace(&ws).map(|w| w.path.clone()))
        .unwrap_or_default();
    let (tab, pane, events) = ctx.model.create_tab(&ws, cwd)?;
    ctx.events.extend(events);
    ctx.pending_spawns.push(pane);
    if let Ok(client) = ctx.view() {
        if ctx.model.client(&client).is_some_and(|c| c.workspace == ws) {
            let events = ctx.model.select_tab(&client, &tab)?;
            ctx.events.extend(events);
        }
    }
    ok(info(ctx, &ws, &tab)?)
}

pub fn rename(ctx: &mut Ctx, p: TabRenameParams) -> Result<Value, ApiError> {
    let tab = ctx.resolve_tab_param(p.tab.as_deref())?;
    match p.name {
        Some(name) => {
            let events = ctx.model.rename_tab(&tab, Some(name))?;
            ctx.events.extend(events);
        }
        None => {
            // No name given: open the prompt in the tab's own cell (interface spec 4.7).
            // Task 18 draws its hints; the cell itself is drawn by tab_row since Task 15.
            let client = ctx.view()?;
            let current = ctx.model.tab(&tab).and_then(|t| t.name.clone());
            let view = ctx
                .model
                .client_mut(&client)
                .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
            view.overlay = Some(Overlay::Prompt(PromptKind::TabName {
                tab,
                input: TextInput::new(current.unwrap_or_default()),
            }));
            view.focus = Focus::Region(RegionKind::Overlay);
        }
    }
    ok(Ack { ok: true })
}

pub fn clear_name(ctx: &mut Ctx, p: TabTargetParams) -> Result<Value, ApiError> {
    let tab = ctx.resolve_tab_param(p.tab.as_deref())?;
    let events = ctx.model.rename_tab(&tab, None)?;
    ctx.events.extend(events);
    ok(Ack { ok: true })
}

pub fn close(ctx: &mut Ctx, p: TabTargetParams) -> Result<Value, ApiError> {
    let tab = ctx.resolve_tab_param(p.tab.as_deref())?;
    let (panes, events) = ctx.model.close_tab(&tab)?;
    ctx.events.extend(events);
    ctx.pending_kills.extend(panes);
    ok(Ack { ok: true })
}

pub fn select(ctx: &mut Ctx, p: TabSelectParams) -> Result<Value, ApiError> {
    let client = ctx.view()?;
    let tab = ctx.resolve_tab_param(Some(&p.tab))?;
    let events = ctx.model.select_tab(&client, &tab)?;
    ctx.events.extend(events);
    ok(Ack { ok: true })
}
