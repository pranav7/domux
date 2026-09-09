#!/bin/bash
# Task 15's mutant set. Each mutant is derived either from a sentence this task's report
# states as a decision (claim-derived, "C") or from the enumerating question - list every
# field the new code reads, and name the input that could make it differ ("E").
#
# Run from the worktree root: ./mutants.sh [name ...]
SWEEP=/Users/pranav/projects/domux-m2/.superpowers/sdd/m2-projects-and-workspaces/sweep
ROOT=$(cd "$(dirname "$0")" && pwd)
NB=crates/domux-server/src/render/name_box.rs
IN=crates/domux-server/src/input.rs
WS=crates/domux-server/src/api/workspace.rs
LS=crates/domux-server/src/api/list.rs

# Scopes. `--test cli` is excluded on purpose: four tests there are wall clock bound and a
# flake reads as a kill.
T_NB="--lib --test name_box"
T_IN="--lib --test name_box --test keys --test prompt_and_help --test switcher --test regions --test copy_mode --test close_tab --test tabs_and_panes"
T_WS="--lib --test name_box --test workspace_focus --test workspace_create --test regions --test switcher --test projects_api --test sidebar"
T_LS="--lib --test name_box --test regions --test switcher --test sidebar --test projects_box"

run() { # name file tests anchor repl
  python3 "$SWEEP" --root "$ROOT" --name "$1" --file "$2" --tests "$3" --anchor "$4" --with "$5"
}

WANT="$*"
pick() { [ -z "$WANT" ] && return 0; case " $WANT " in *" $1 "*) return 0;; esac; return 1; }

# --- controls ------------------------------------------------------------------
pick CONTROL-survive && python3 "$SWEEP" --root "$ROOT" --control-survive --tests "$T_NB"
pick CONTROL-die && python3 "$SWEEP" --root "$ROOT" --control-die --name CONTROL-die --file "$NB" \
  --tests "$T_NB" --anchor 'const WIDTH: u16 = 58;' --with 'const WIDTH: u16 = 40;'

# --- render: the box itself ----------------------------------------------------
# C: the title carries the handle, so the reader knows which slot they are naming.
pick M01 && run M01 "$NB" "$T_NB" \
  'overlay::frame_at(&format!("Name {}", w.handle), area, buf)' \
  'overlay::frame_at(&format!("Name {}", w.display_name()), area, buf)'
# E: the hints sit under the field, not on it.
pick M02 && run M02 "$NB" "$T_NB" 'const HINTS_ROW: u16 = 2;' 'const HINTS_ROW: u16 = 1;'
# E: the box is 5 rows, which is what puts its border on row 10 of an 80x24 screen.
pick M03 && run M03 "$NB" "$T_NB" 'const HEIGHT: u16 = 5;' 'const HEIGHT: u16 = 7;'
# C: the caret sits on the cursor, not at the end of the text.
pick M04 && run M04 "$NB" "$T_NB" '        let cursor = input.view.input.cursor;' \
  '        let cursor = input.view.input.text.chars().count();'
# E: the caret reverses the character it is on rather than always a space.
pick M05 && run M05 "$NB" "$T_NB" '        let rest: String = after.chars().skip(1).collect();' \
  '        let caret = " ".to_string();
        let rest: String = after.clone();'
# E: the field is drawn one cell in from the border, under the title.
pick M06 && run M06 "$NB" "$T_NB" \
  '        let mut cx = put_within(buf, inner.x + 1, inner.y, last_x, &before, text);' \
  '        let mut cx = put_within(buf, inner.x, inner.y, last_x, &before, text);'
# C: a pill takes the hint row while one is showing.
pick M07 && run M07 "$NB" "$T_NB" '                    &truncate_with_ellipsis(&pill.text, budget),' '                    "",'
# C: a refused pill reads as refused.
pick M08 && run M08 "$NB" "$T_NB" '.bg(if pill.ok { theme::GREEN } else { theme::RED })' '.bg(theme::GREEN)'
# C: the hint line says what the two keys do and what an empty name means.
pick M09 && run M09 "$NB" "$T_NB" '                    ("an empty name clears it", quiet),' '                    ("", quiet),'
# E: the budget is the inside less its padding, not nothing.
pick M10 && run M10 "$NB" "$T_NB" '                let mut left = budget;' '                let mut left = 0;'
# C: the box covers what was under it.
pick M11 && run M11 "$NB" "$T_NB" \
  '    let inner = overlay::frame_at(&format!("Name {}", w.handle), area, buf);' \
  '    let inner = crate::render::boxed::Boxed { title: &format!("Name {}", w.handle), flag: None, focused: true }.render(area, buf);'
# C: the screen behind reads as being behind it, and the box does not.
pick M12 && run M12 "$NB" "$T_NB" '    overlay::dim(buf, &[area]);' '    overlay::dim(buf, &[]);'
# C: a workspace the model no longer holds draws no box.
pick M13 && run M13 "$NB" "$T_NB" \
  '    let Some(w) = input.model.workspace(workspace) else {
        return;
    };' \
  '    let w = input.model.workspace(workspace);
    let w = w.cloned().unwrap_or_else(|| domux_core::model::Workspace {
        id: workspace.clone(),
        handle: domux_core::model::WorkspaceHandle::Main,
        name: None,
        path: "/".into(),
        tabs: Vec::new(),
        last_tab: None,
    });'

# --- input: the keys the box takes ---------------------------------------------
# C: Esc goes back one overlay rather than stranding what was underneath.
pick M20 && run M20 "$IN" "$T_IN" '                Key::Escape => close_top_overlay(core, client),' \
  '                Key::Escape => close_overlay(core, client),'
# E: Esc closes at all.
pick M21 && run M21 "$IN" "$T_IN" '                Key::Escape => close_top_overlay(core, client),' \
  '                Key::Escape => {}'
# C: the box closes only when the rename worked.
pick M22 && run M22 "$IN" "$T_IN" '        Err(e) => core.set_pill(Some(client), e.message, false),' \
  '        Err(e) => {
            close_top_overlay(core, client);
            core.set_pill(Some(client), e.message, false)
        }'
# C: a refusal answers where the reader is looking.
pick M23 && run M23 "$IN" "$T_IN" '        Err(e) => core.set_pill(Some(client), e.message, false),' '        Err(_) => {}'
# C: the pill names the slot it named.
pick M24 && run M24 "$IN" "$T_IN" '                format!("Named {handle} {name}")' '                format!("Named {name}")'
# C: a blank name reports as a clear, agreeing with what the model did.
pick M25 && run M25 "$IN" "$T_IN" '            let name = name.trim();' '            let name = name.as_str();'
# E: the pill is set at all after a save.
pick M26 && run M26 "$IN" "$T_IN" '            core.set_pill(Some(client), text, true);' '            let _ = text;'
# C: a key in the box clears the last result.
pick M27 && run M27 "$IN" "$T_IN" \
  '            if let Some(view) = core.model.client_mut(client) {
                view.pill = None;
            }' '            {}'
# C: a chorded letter is not text.
pick M28 && run M28 "$IN" "$T_IN" \
  '        Key::Char(c) if !key.mods.intersects(Mods::CTRL | Mods::ALT) => view.input.insert(c),' \
  '        Key::Char(c) => view.input.insert(c),'
# C: a key the box has no meaning for does nothing.
pick M29 && run M29 "$IN" "$T_IN" '                _ => edit_name(core, client, &key),' \
  '                _ => close_top_overlay(core, client),'
# E: Enter saves what is in the field.
pick M30 && run M30 "$IN" "$T_IN" '        core.model.client(client).map(|v| v.input.text.clone()),' '        Some(String::new()),'
# C: Enter names the workspace the box is for, not the one the client is in.
pick M31 && run M31 "$IN" "$T_IN" '        workspace: Some(workspace.to_string()),' '        workspace: None,'
# E: Enter closes the box on success.
pick M32 && run M32 "$IN" "$T_IN" '            close_top_overlay(core, client);
            let name = name.trim();' '            let name = name.trim();'
# E: Backspace edits the field.
pick M33 && run M33 "$IN" "$T_IN" '        Key::Backspace => view.input.backspace(),' '        Key::Backspace => {}'

# --- api::workspace: opening the box -------------------------------------------
# C: the box opens over what is there rather than replacing it.
pick M40 && run M40 "$WS" "$T_WS" '        view.push_overlay(Overlay::NameWorkspace(target));' \
  '        view.overlay = Some(Overlay::NameWorkspace(target));'
# C: a second open replaces rather than stacks.
pick M41 && run M41 "$WS" "$T_WS" '    if matches!(view.overlay, Some(Overlay::NameWorkspace(_))) {' '    if false {'
# C: the box opens on the name the workspace has.
pick M42 && run M42 "$WS" "$T_WS" '    view.input = TextInput::new(current);' '    view.input = TextInput::new("");'
# C: the hint row that opens says what the keys do, not what happened before.
pick M43 && run M43 "$WS" "$T_WS" '    view.pill = None;
' ''
# C: the keys are marked as being in the box.
pick M44 && run M44 "$WS" "$T_WS" '    view.focus = Focus::Region(RegionKind::Overlay);
' ''
# C: no name opens the box rather than changing anything.
pick M45 && run M45 "$WS" "$T_WS" '        return open_name_box(ctx, target);' \
  '        return Err(ApiError::unavailable("give a name"));'
# C: a call for a client that is not attached opens nothing.
pick M46 && run M46 "$WS" "$T_WS" \
  '    let view = ctx
        .model
        .client_mut(&client)
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    view.input = TextInput::new(current);' \
  '    let Some(view) = ctx.model.client_mut(&client) else {
        return ok(Ack { ok: true });
    };
    view.input = TextInput::new(current);'
# C: the cursor row is the target only while the keys are in a box.
pick M47 && run M47 "$WS" "$T_WS" '            Some(cursor) if super::list::in_a_box(self, &client) => Ok(cursor.clone()),' \
  '            Some(cursor) => Ok(cursor.clone()),'
# C: the cursor row is the target when they are.
pick M48 && run M48 "$WS" "$T_WS" '            Some(cursor) if super::list::in_a_box(self, &client) => Ok(cursor.clone()),' \
  '            Some(_) if false => unreachable!(),'

# --- api::list: which box has the keys -----------------------------------------
pick M50 && run M50 "$LS" "$T_LS" '    surface(ctx, client).is_ok()' '    true'
pick M51 && run M51 "$LS" "$T_LS" '    surface(ctx, client).is_ok()' '    false'

# --- round two: derived after the first sweep found no survivors, from the fields and
# --- branches the first round's tests were not shaped around --------------------
# E: the hint line loses its last piece with an ellipsis rather than a hard clip.
pick M60 && run M60 "$NB" "$T_NB" '                    let piece = truncate_with_ellipsis(piece, left);' \
  '                    let piece = piece.to_string();'
# E: the budget is the inside less its two pads.
pick M61 && run M61 "$NB" "$T_NB" '    let budget = inner.width.saturating_sub(2) as usize;' \
  '    let budget = inner.width as usize;'
# E: the hints only draw when there is a row inside the border for them.
pick M62 && run M62 "$NB" "$T_NB" '    if inner.height > HINTS_ROW {' '    if true {'
# E: the field only draws when there is a row inside the border for it.
pick M63 && run M63 "$NB" "$T_NB" '    if inner.height > 0 {' '    if true {'
# E: the text after the caret is drawn too.
pick M64 && run M64 "$NB" "$T_NB" '        put_within(buf, cx, inner.y, last_x, &rest, text);' \
  '        put_within(buf, cx, inner.y, last_x, "", text);'
# E: each caret key moves the caret.
pick M65 && run M65 "$IN" "$T_IN" '        Key::Left => view.input.left(),' '        Key::Left => {}'
pick M66 && run M66 "$IN" "$T_IN" '        Key::Right => view.input.right(),' '        Key::Right => {}'
pick M67 && run M67 "$IN" "$T_IN" '        Key::Home => view.input.home(),' '        Key::Home => {}'
pick M68 && run M68 "$IN" "$T_IN" '        Key::End => view.input.end(),' '        Key::End => {}'
# E: the field opens on the name, not on the handle standing in for one.
pick M69 && run M69 "$WS" "$T_WS" '        .and_then(|w| w.name.clone())' '        .map(|w| w.display_name())'
# E: "the keys are in a box" covers the sidebar's box as well as the switcher's.
pick M70 && run M70 "$LS" "$T_LS" '    surface(ctx, client).is_ok()' \
  '    ctx.model.client(client).map(|v| v.overlay == Some(domux_core::model::Overlay::Switcher)).unwrap_or(false)'
# E: the box is where `centred_area` puts it, to the cell.
pick M71 && run M71 "$NB" "$T_NB" 'const WIDTH: u16 = 58;' 'const WIDTH: u16 = 60;'
# E: the caret takes the width of the character under it.
pick M72 && run M72 "$NB" "$T_NB" '            .filter(|s| !s.is_empty())' '            .filter(|_| false)'
