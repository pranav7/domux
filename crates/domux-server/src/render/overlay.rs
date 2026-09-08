//! Overlays drawn over the workpanel. M1: the keys help. The tab-name prompt is drawn by
//! `tab_row` in the tab's cell. M2 adds NameWorkspace and Confirm; M4 adds Usage.

use crate::render::boxed::{put_within, Boxed};
use crate::render::{theme, RenderInput};
use domux_core::model::Overlay;
use domux_core::text::truncate_with_ellipsis;
use domux_term::Size;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    // Only the help overlay draws here in M1. The prompt lives in the tab cell; the others
    // arrive with M2 to M4.
    if let Some(Overlay::Help) = &input.view.overlay {
        draw_help(input, buf);
    }
}

/// The centred box every overlay draws into (roadmap 5.1): `width` by `height` cells,
/// clamped to the workpanel, cleared to `base`, with a focused `Boxed` titled `title`. Returns
/// the inner area. M2's switcher, name-workspace and confirmation overlays call this.
///
/// Centred over the workpanel, not over the whole screen. The top bar carries the tab row and
/// the clock and stays readable under every overlay, and a box centred over the screen puts its
/// own border on the pane box's border row whenever the two heights are close: `┌ ┌ Keys ─┐─┐`
/// on an 80x24 or 40x10 screen, which reads as one broken box rather than two boxes
/// (principle 14).
///
/// Clamped twice: to the wanted size less a margin, and then to the workpanel itself.
/// `Boxed::render` indexes the buffer by row without checking it, so an area one row past the
/// edge panics rather than clips, and a workpanel smaller than the smallest box gets no box at
/// all rather than one drawn outside itself.
pub fn frame(title: &str, width: u16, height: u16, buf: &mut Buffer) -> Rect {
    let screen = buf.area;
    // The whole width, sidebar or no sidebar: an overlay is the one thing on the screen the
    // keys go to, so it is centred on the screen and covers the sidebar like anything else.
    let workpanel = crate::render::workpanel_of(
        Size {
            cols: screen.width,
            rows: screen.height,
        },
        false,
    );
    let panel = Rect::new(
        screen.x + workpanel.x,
        screen.y + workpanel.y,
        workpanel.width,
        workpanel.height,
    );
    let width = width
        .min(panel.width.saturating_sub(4))
        .max(20)
        .min(panel.width);
    let height = height
        .min(panel.height.saturating_sub(2))
        .max(3)
        .min(panel.height);
    if width == 0 || height == 0 {
        return Rect::new(panel.x, panel.y, 0, 0);
    }
    let area = Rect::new(
        panel.x + (panel.width - width) / 2,
        panel.y + (panel.height - height) / 2,
        width,
        height,
    );
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf[(x, y)].reset();
            buf[(x, y)].set_style(Style::default().bg(theme::BASE));
        }
    }
    Boxed {
        title,
        flag: None,
        focused: true,
    }
    .render(area, buf)
}

/// `┌ Keys ┐`: the leader, every `[keys.bindings]` line as `C-a |    pane.split right`,
/// every `[keys.global]` line, the passthrough rule, and `esc close`. Rendered from the
/// loaded keymap, so a rebinding shows here (principle 3).
fn draw_help(input: &RenderInput, buf: &mut Buffer) {
    let km = input.keymap;
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("leader {}", km.leader));
    lines.push(String::new());
    // Collapse the run of `tab.select <n>` bindings into one row. Nine near-identical rows
    // push the globals, the passthrough rule and the footer past the bottom of the box on
    // an 80x24 screen, and the row still renders the configured keys (principle 3).
    let mut digits: Vec<String> = km
        .bindings
        .iter()
        .filter(|b| b.action.to_string().starts_with("tab.select "))
        .map(|b| b.key.to_string())
        .collect();
    digits.sort();
    let mut bindings: Vec<(String, String)> = km
        .bindings
        .iter()
        .filter(|b| !b.action.to_string().starts_with("tab.select "))
        .map(|b| (format!("{} {}", km.leader, b.key), b.action.to_string()))
        .collect();
    if let (Some(first), Some(last)) = (digits.first(), digits.last()) {
        let keys = if digits.len() == 1 {
            first.clone()
        } else {
            format!("{first}-{last}")
        };
        bindings.push((format!("{} {}", km.leader, keys), "tab.select <n>".into()));
    }
    bindings.sort_by(|a, b| a.1.cmp(&b.1));
    let mut globals: Vec<(String, String)> = km
        .global
        .iter()
        .map(|b| (b.key.to_string(), b.action.to_string()))
        .collect();
    globals.sort_by(|a, b| a.1.cmp(&b.1));
    for (k, a) in bindings.iter().chain(globals.iter()) {
        lines.push(format!("{k:<10} {a}"));
    }
    if !km.passthrough_commands.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "{} keep {}",
            km.passthrough_commands.join(", "),
            km.passthrough_keys
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lines.push(String::new());
    let inner = frame("Keys", 60, lines.len() as u16 + 3, buf);
    if inner.width < 3 || inner.height == 0 {
        return;
    }
    // The footer owns the last inner row whatever else fits, so the overlay always shows the
    // way out of itself (principle 9). On a screen too short for every binding the list
    // truncates and says so; it never silently swallows `esc close`.
    let body_rows = inner.height.saturating_sub(1) as usize;
    let truncated = lines.len() > body_rows;
    let shown = if truncated {
        body_rows.saturating_sub(1)
    } else {
        lines.len()
    };
    let width = inner.width.saturating_sub(2) as usize;
    let last_x = inner.x + inner.width - 1;
    let text = Style::default().fg(theme::TEXT).bg(theme::BASE);
    for (i, line) in lines.iter().take(shown).enumerate() {
        put_within(
            buf,
            inner.x + 1,
            inner.y + i as u16,
            last_x,
            &truncate_with_ellipsis(line, width),
            text,
        );
    }
    if truncated {
        let more = lines.len() - shown;
        let note = format!("{more} more, see domux.toml");
        let style = Style::default().fg(theme::SUBTEXT0).bg(theme::BASE);
        put_within(
            buf,
            inner.x + 1,
            inner.y + shown as u16,
            last_x,
            &truncate_with_ellipsis(&note, width),
            style,
        );
    }
    let footer = Style::default().fg(theme::BLUE).bg(theme::BASE);
    put_within(
        buf,
        inner.x + 1,
        inner.y + inner.height - 1,
        last_x,
        "esc close",
        footer,
    );
}
