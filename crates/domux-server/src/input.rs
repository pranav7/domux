//! Key routing in the architecture spec's order: overlay, leader chord, global with
//! passthrough, then the focus target.
//!
//! Design principle 1: the pane owns its input. Only three things claim a key ahead of it -
//! an open overlay, a chord, a global binding the foreground command does not pass through -
//! and everything else reaches the pane's emulator unchanged. The leader pressed twice sends
//! the leader itself, so a program that wants `C-a` can still have it.

use crate::client::Hint;
use crate::copy_mode::{self, CopyOutcome};
use crate::core::Core;
use domux_core::api::Method;
use domux_core::ids::{ClientId, PaneId};
use domux_core::keymap::Action;
use domux_core::model::{Chord, ConfirmKind, Focus, Overlay, PromptKind};
use domux_core::proto::ServerMsg;
use domux_term::{Emulator, Key, KeyAction, KeyEvent, Mods};

/// Where one key went. Returned for tests and logs; routing itself is the side effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Overlay,
    Chord,
    Global(Action),
    Pane,
}

pub fn route_key(core: &mut Core, client: &ClientId, key: KeyEvent) -> Route {
    if key.action == KeyAction::Release {
        return Route::Pane;
    }
    // Read what decides the route, not the whole view: a key press is the hottest path in
    // the server and nothing below needs the rest of it.
    let Some((has_overlay, in_chord)) = core
        .model
        .client(client)
        .map(|view| (view.overlay.is_some(), view.chord.is_some()))
    else {
        return Route::Pane;
    };

    // 1. An open overlay takes every key.
    if has_overlay {
        overlay_key(core, client, key);
        return Route::Overlay;
    }

    // 2. A chord in progress. It ends on this key whatever the key is: the indicator
    //    disappearing is the visible answer to an unbound one.
    if in_chord {
        if let Some(v) = core.model.client_mut(client) {
            v.chord = None;
        }
        if core.config.keymap.leader.matches(&key) {
            forward_to_pane(core, client, &key);
        } else if let Some(action) = core.config.keymap.binding_for(&key).cloned() {
            core.run_action(client, &action);
        }
        return Route::Chord;
    }

    if core.config.keymap.leader.matches(&key) {
        let leader = core.config.keymap.leader.to_string();
        if let Some(v) = core.model.client_mut(client) {
            v.chord = Some(Chord { leader });
        }
        return Route::Chord;
    }

    // 3. Global bindings, unless the foreground command passes this key through. An unknown
    //    foreground passes nothing through (plan assumption 22): domux keeps the key rather
    //    than dropping it into a pane whose program it has not identified yet.
    let foreground = core
        .focused_pane(client)
        .and_then(|p| core.model.pane(&p).and_then(|x| x.command.clone()));
    let global = core
        .config
        .keymap
        .global_for(&key, foreground.as_deref())
        .cloned();
    if let Some(action) = global {
        core.run_action(client, &action);
        return Route::Global(action);
    }

    // 4. The focus target. A pane in copy mode handles the key itself; a pane whose child
    //    exited (terminal.remain_on_exit) closes on Enter and swallows other keys.
    //
    //    Copy mode first, and on an exited pane too (ruled 2026-09-07). While it is open the
    //    bar reads `⏎ copy · esc leave`, and with the exited branch ahead of it Esc did
    //    nothing, there was no way out of the mode, and Enter closed the pane: a key doing
    //    something destructive that its own visible label says it does not do (principles 3
    //    and 10). An exited pane's scrollback is also exactly what a reader wants to copy.
    if let Some(pane) = core.focused_pane(client) {
        if core.panes.get(&pane).is_some_and(|rt| rt.copy.is_some()) {
            let outcome = {
                let rt = core.panes.get_mut(&pane).expect("pane runtime");
                copy_mode::handle_key(rt, &key)
            };
            match outcome {
                CopyOutcome::Continue => {}
                CopyOutcome::Copy(text) => {
                    if let Some(conn) = core.clients.get(client) {
                        let _ = conn.tx.try_send(ServerMsg::Clipboard(text));
                    }
                    leave_copy_mode(core, &pane);
                }
                CopyOutcome::NotCopied(why) => {
                    // An action hint: it answers this key and is gone on the next one.
                    if let Some(conn) = core.clients.get_mut(client) {
                        conn.hint = Some(Hint::action(why));
                    }
                    leave_copy_mode(core, &pane);
                }
                CopyOutcome::Leave => leave_copy_mode(core, &pane),
            }
            return Route::Pane;
        }
        if core.panes.get(&pane).is_some_and(|rt| rt.exited.is_some()) {
            if key.key == Key::Enter {
                // Not `close_pane`: closing the workspace's last pane starts a replacement
                // shell, and the respawn guard has to bound that however it is reached.
                core.close_exited_pane(&pane);
            }
            return Route::Pane;
        }
    }
    forward_to_pane(core, client, &key);
    Route::Pane
}

fn leave_copy_mode(core: &mut Core, pane: &PaneId) {
    if let Some(rt) = core.panes.get_mut(pane) {
        rt.copy = None;
        rt.dirty = true;
    }
    core.model.set_pane_copy_mode(pane, false);
}

fn forward_to_pane(core: &mut Core, client: &ClientId, key: &KeyEvent) {
    if let Some(pane) = core.focused_pane(client) {
        if let Some(rt) = core.panes.get_mut(&pane) {
            let mut out = Vec::new();
            rt.emulator.encode_key(key, &mut out);
            if !out.is_empty() {
                rt.write(&out);
            }
        }
    }
}

/// Keys inside an overlay. The prompt edits its input; Enter saves, Esc cancels. The help
/// overlay closes on Esc, `q` or `?`. A confirmation acts on `y` and cancels on anything
/// else. Closing returns focus to the pane.
fn overlay_key(core: &mut Core, client: &ClientId, key: KeyEvent) {
    let Some(overlay) = core
        .model
        .client(client)
        .and_then(|view| view.overlay.clone())
    else {
        return;
    };
    match overlay {
        Overlay::Help => {
            if matches!(key.key, Key::Escape | Key::Char('q') | Key::Char('?')) {
                close_overlay(core, client);
            }
        }
        Overlay::Prompt(PromptKind::TabName { tab, mut input }) => {
            match key.key {
                Key::Escape => return close_overlay(core, client),
                Key::Enter => {
                    close_overlay(core, client);
                    let method = Method::TabRename(domux_core::api::TabRenameParams {
                        tab: Some(tab.to_string()),
                        name: Some(input.text),
                        client: Some(client.clone()),
                    });
                    let _ = core.dispatch(method, Some(client.clone()));
                    return;
                }
                Key::Backspace => input.backspace(),
                Key::Left => input.left(),
                Key::Right => input.right(),
                Key::Home => input.home(),
                Key::End => input.end(),
                Key::Char(c) if !key.mods.contains(Mods::CTRL) && !key.mods.contains(Mods::ALT) => {
                    input.insert(c)
                }
                _ => return,
            }
            set_prompt(core, client, PromptKind::TabName { tab, input });
        }
        // `y` and nothing else closes the tab. Any other key cancels rather than waiting for
        // one of two right answers: the safe outcome is the one a stray keystroke should
        // reach, and a reader who typed something else has already stopped reading the
        // question. Esc and `n` are in that set, and are what the bar offers.
        Overlay::Confirm(ConfirmKind::CloseTab(tab)) => {
            close_overlay(core, client);
            if matches!(key.key, Key::Char('y') | Key::Char('Y')) {
                let method = Method::TabClose(domux_core::api::TabTargetParams {
                    tab: Some(tab.to_string()),
                    client: Some(client.clone()),
                });
                let _ = core.dispatch(method, Some(client.clone()));
            }
        }
        // Task 14 replaces this with the whole of `[keys.list]`, the routing the sidebar's
        // box and M3's agents overlay share. Until then only the key bound to `focus.pane`
        // is read, so the switcher has a way out and it is the configured one (principle 3).
        Overlay::Switcher => {
            let action = core.config.keymap.list_for(&key).cloned();
            if action.is_some_and(|a| a.method == "focus.pane") {
                let method = Method::SwitcherClose(domux_core::api::ClientParams {
                    client: Some(client.clone()),
                });
                let _ = core.dispatch(method, Some(client.clone()));
            }
        }
        Overlay::Agents
        | Overlay::NameWorkspace(_)
        | Overlay::Confirm(ConfirmKind::DeleteWorkspace(_) | ConfirmKind::RemoveProject(_))
        | Overlay::Usage => {
            // M1 never opens these. M3 and M4 add their key handling here.
        }
    }
}

fn set_prompt(core: &mut Core, client: &ClientId, prompt: PromptKind) {
    if let Some(view) = core.model.client_mut(client) {
        view.overlay = Some(Overlay::Prompt(prompt));
    }
}

fn close_overlay(core: &mut Core, client: &ClientId) {
    let focused = core.focused_pane(client);
    if let Some(view) = core.model.client_mut(client) {
        view.overlay = None;
        if let Some(p) = focused {
            view.focus = Focus::Pane(p);
        }
    }
}
