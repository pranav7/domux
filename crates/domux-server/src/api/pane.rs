//! `pane.*`

use super::{ok, Ctx};
use crate::copy_mode::CopyMode;
use domux_core::api::{
    Ack, ApiError, PaneInfo, PaneReadParams, PaneReadResult, PaneResizeParams, PaneSendKeyParams,
    PaneSendTextParams, PaneSplitParams, PaneTargetParams, TabTargetParams, ZoomResult,
};
use domux_core::keymap::KeyName;
use domux_term::{Emulator, KeyAction, KeyEvent, ScrollbackPos};
use serde_json::Value;

pub fn list(ctx: &mut Ctx, p: TabTargetParams) -> Result<Value, ApiError> {
    let tab = ctx.resolve_tab_param(p.tab.as_deref())?;
    let ids = ctx
        .model
        .tab(&tab)
        .map(|t| t.layout.pane_ids())
        .unwrap_or_default();
    let infos: Vec<PaneInfo> = ids.iter().filter_map(|id| ctx.pane_info(id)).collect();
    ok(infos)
}

pub fn split(ctx: &mut Ctx, p: PaneSplitParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let cwd = p
        .cwd
        .clone()
        .or_else(|| ctx.model.pane(&pane).map(|x| x.cwd.clone()))
        .unwrap_or_default();
    let (new, events) = ctx.model.split_pane(&pane, p.dir, cwd)?;
    ctx.events.extend(events);
    ctx.pending_spawns.push(new.clone());
    ctx.view_dirty = true;
    // The new pane has no runtime yet - the core spawns it after this returns - so its
    // size reads 0x0 here. `focused` is the one field the split itself settles.
    let mut info = ctx
        .pane_info(&new)
        .ok_or_else(|| ApiError::internal("the new pane vanished"))?;
    info.focused = true;
    ok(info)
}

pub fn close(ctx: &mut Ctx, p: PaneTargetParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let (panes, _, events) = ctx.model.close_pane(&pane)?;
    ctx.events.extend(events);
    ctx.pending_kills.extend(panes);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

pub fn focus(ctx: &mut Ctx, p: PaneTargetParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let loc = ctx
        .model
        .pane_location(&pane)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} does not exist")))?;
    let client = ctx.view()?;
    if ctx.model.client(&client).is_some_and(|c| c.tab != loc.tab) {
        let events = ctx.model.select_tab(&client, &loc.tab)?;
        ctx.events.extend(events);
    }
    let events = ctx.model.focus_pane(&pane)?;
    ctx.events.extend(events);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

pub fn zoom(ctx: &mut Ctx, p: PaneTargetParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let loc = ctx
        .model
        .pane_location(&pane)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} does not exist")))?;
    if ctx
        .model
        .tab(&loc.tab)
        .map(|t| t.focused != pane)
        .unwrap_or(true)
    {
        let events = ctx.model.focus_pane(&pane)?;
        ctx.events.extend(events);
    }
    let events = ctx.model.toggle_zoom(&loc.tab)?;
    ctx.events.extend(events);
    ctx.view_dirty = true;
    ok(ZoomResult {
        zoomed: ctx.model.tab(&loc.tab).and_then(|t| t.zoomed.clone()),
    })
}

/// Enters copy mode on the pane, or leaves it when already in it. Task 19 gives `CopyMode`
/// its keys; here it only exists.
pub fn copy_mode(ctx: &mut Ctx, p: PaneTargetParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let rt = ctx
        .panes
        .get_mut(&pane)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} has no terminal")))?;
    if rt.copy.take().is_none() {
        let cursor = rt.emulator.cursor();
        rt.copy = Some(CopyMode::new(rt.emulator.size(), (cursor.row, cursor.col)));
    }
    let on = rt.copy.is_some();
    rt.dirty = true;
    ctx.model.set_pane_copy_mode(&pane, on);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

pub fn resize(ctx: &mut Ctx, p: PaneResizeParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let loc = ctx
        .model
        .pane_location(&pane)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} does not exist")))?;
    let area = ctx.smallest_area(&loc.tab);
    // The bool `resize_pane` returns says an ancestor split owned the axis, not that the
    // geometry moved, so it is deliberately not reported. `Ack.ok` means the call ran.
    ctx.model.resize_pane(&pane, p.dir, p.cells, area)?;
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// Writes the text as typed. `\n` becomes `\r`, which is what Enter sends.
pub fn send_text(ctx: &mut Ctx, p: PaneSendTextParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let rt = ctx
        .panes
        .get_mut(&pane)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} has no terminal")))?;
    rt.write(p.text.replace('\n', "\r").as_bytes());
    ok(Ack { ok: true })
}

pub fn send_key(ctx: &mut Ctx, p: PaneSendKeyParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let name = KeyName::parse(&p.key).map_err(ApiError::invalid_params)?;
    let rt = ctx
        .panes
        .get_mut(&pane)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} has no terminal")))?;
    let mut out = Vec::new();
    rt.emulator.encode_key(
        &KeyEvent {
            key: name.key,
            mods: name.mods,
            action: KeyAction::Press,
        },
        &mut out,
    );
    rt.write(&out);
    ok(Ack { ok: true })
}

/// The last `lines` lines ending at the cursor's row (default: as many as the screen has
/// rows), scrollback included. Rows below the cursor are blank and are not content.
pub fn read(ctx: &mut Ctx, p: PaneReadParams) -> Result<Value, ApiError> {
    let pane = ctx.resolve_pane_param(p.pane.as_deref())?;
    let rt = ctx
        .panes
        .get_mut(&pane)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} has no terminal")))?;
    let size = rt.emulator.size();
    let end = rt.emulator.scrollback_len() + rt.emulator.cursor().row as usize;
    let lines = p.lines.unwrap_or(size.rows as usize).max(1).min(end + 1);
    let text = rt.emulator.text_in_range(
        ScrollbackPos {
            row: end + 1 - lines,
            col: 0,
        },
        ScrollbackPos {
            row: end,
            col: size.cols.saturating_sub(1),
        },
    );
    ok(PaneReadResult {
        text: text.trim_end_matches('\n').to_string(),
    })
}
