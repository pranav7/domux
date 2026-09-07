//! The full-width top bar: `project › workspace`, the tab row, `+`, and at the right end the
//! clock or whatever displaces it: prompt keys, copy mode keys, the leader indicator, a
//! client hint, the config error.

use crate::render::boxed::put;
use crate::render::tab_row::draw_tabs;
use crate::render::{theme, RenderInput};
use domux_core::model::Overlay;
use domux_core::text::display_width;
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};

/// A run of text with one style, for the right end.
pub struct Piece {
    pub text: String,
    pub style: Style,
}

pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    // The buffer is the authority on how wide the bar may be, not the client's reported
    // size: the fill below indexes cells directly, so a width taken from anywhere else
    // would panic the moment the two disagreed.
    let area = buf.area;
    if area.height == 0 {
        return;
    }
    let y = area.y;
    let right_edge = area.x + area.width;
    let bg = theme::MANTLE;
    for x in area.x..right_edge {
        buf[(x, y)].reset();
        buf[(x, y)].set_style(Style::default().bg(bg));
    }
    let Some(ws) = input.model.workspace(&input.view.workspace) else {
        return;
    };
    let project = input
        .model
        .project_of_workspace(&ws.id)
        .map(|p| p.name.as_str())
        .unwrap_or("");
    let location = format!(" {project} › {} ", ws.display_name());
    let mut x = put(
        buf,
        area.x,
        y,
        &location,
        Style::default()
            .fg(theme::TEXT)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    );
    let current = ws
        .tabs
        .iter()
        .position(|t| t.id == input.view.tab)
        .unwrap_or(0);
    let prompt = match &input.view.overlay {
        Some(Overlay::Prompt(p)) => Some(p),
        _ => None,
    };
    x = draw_tabs(&ws.tabs, current, prompt, x, y, buf);
    let pieces = right_pieces(input);
    // The trailing cell keeps the right end off the last column, so the bar reads as a bar
    // rather than as text pressed against the screen edge.
    let total: usize = pieces.iter().map(|p| display_width(&p.text)).sum::<usize>() + 1;
    let from_left = area.x as usize + (area.width as usize).saturating_sub(total);
    // Never before the tab row's own end, and never past the buffer: `put` clips too, but a
    // start beyond `u16` would wrap in the cast before it ever got there.
    let start = from_left.max(x as usize + 1).min(right_edge as usize) as u16;
    let mut cx = start;
    for p in pieces {
        cx = put(buf, cx, y, &p.text, p.style.bg(bg));
    }
}

/// What the right end shows, in priority order. Tasks 18, 19 and 21 add their pieces.
pub fn right_pieces(input: &RenderInput) -> Vec<Piece> {
    let dim = Style::default().fg(theme::OVERLAY0);
    if let Some(hint) = input.hint {
        return vec![Piece {
            text: hint.to_string(),
            style: dim,
        }];
    }
    vec![Piece {
        text: input.now.format("%H:%M   %a %-d %b").to_string(),
        style: Style::default().fg(theme::SUBTEXT0),
    }]
}
