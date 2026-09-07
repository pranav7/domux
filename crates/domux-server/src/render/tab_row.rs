//! Tab cells: `1` or `2 pr1`, the current tab filled accent, `│` separators, `+`.

use crate::render::boxed::put;
use crate::render::theme;
use domux_core::model::{PromptKind, Tab};
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};

/// Draws the tab cells from `x` on row `y` and returns the x after the trailing separator.
/// When `prompt` names the current tab, its cell shows the prompt instead of its name
/// (interface spec 4.7): `Name tab 2 › input▮`.
///
/// Every write goes through `put`, which clips at the buffer's right edge, so a tab row
/// wider than the screen is truncated rather than drawn past it.
pub fn draw_tabs(
    tabs: &[Tab],
    current: usize,
    prompt: Option<&PromptKind>,
    x: u16,
    y: u16,
    buf: &mut Buffer,
) -> u16 {
    let bg = theme::MANTLE;
    let sep = Style::default().fg(theme::SURFACE0).bg(bg);
    let mut cx = x;
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            cx = put(buf, cx, y, "│", sep);
        }
        let is_current = i == current;
        if let (true, Some(prompt)) = (is_current, prompt) {
            cx = draw_prompt_cell(prompt, i + 1, cx, y, buf);
            continue;
        }
        let label = match &tab.name {
            Some(name) => format!(" {} {} ", i + 1, name),
            None => format!(" {} ", i + 1),
        };
        let style = if is_current {
            Style::default()
                .fg(theme::BASE)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::OVERLAY1).bg(bg)
        };
        cx = put(buf, cx, y, &label, style);
    }
    cx = put(buf, cx, y, "│", sep);
    cx = put(
        buf,
        cx,
        y,
        " + ",
        Style::default().fg(theme::SURFACE2).bg(bg),
    );
    put(buf, cx, y, "│", sep)
}

fn draw_prompt_cell(prompt: &PromptKind, number: usize, x: u16, y: u16, buf: &mut Buffer) -> u16 {
    let PromptKind::TabName { input, .. } = prompt;
    let fill = Style::default().fg(theme::BASE).bg(theme::ACCENT);
    let label = fill.add_modifier(Modifier::DIM);
    let mut cx = put(buf, x, y, &format!(" Name tab {number} › "), label);
    let before: String = input.text.chars().take(input.cursor).collect();
    let after: String = input.text.chars().skip(input.cursor).collect();
    cx = put(buf, cx, y, &before, fill);
    // The caret: reverse the cell under it so it reads without colour.
    let caret = after
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    cx = put(buf, cx, y, &caret, fill.add_modifier(Modifier::REVERSED));
    let rest: String = after.chars().skip(1).collect();
    cx = put(buf, cx, y, &rest, fill);
    put(buf, cx, y, " ", fill)
}
