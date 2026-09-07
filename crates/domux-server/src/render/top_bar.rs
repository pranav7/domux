//! The full-width top bar: `project › workspace`, the tab row, `+`, and at the right end the
//! clock or whatever displaces it: prompt keys, copy mode keys, the leader indicator, a
//! client hint, the config error.

use crate::render::boxed::{put, put_within};
use crate::render::tab_row::TabRow;
use crate::render::{theme, RenderInput};
use domux_core::model::Overlay;
use domux_core::text::{display_width, truncate_to_width};
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
    let x = put(
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
    // The room the tab row and the right end share, and how they share it. The tab row keeps
    // enough of it for the current tab, which has to be visible on every frame (principle 2);
    // the right end takes what it wants from the rest and elides into it when that is not
    // enough, rather than running off the screen edge (principle 6). Neither may reach the
    // last column: an empty cell there keeps the bar reading as a bar rather than as text
    // pressed against the edge, and one more cell keeps an elided right end off the tab row.
    let room = right_edge.saturating_sub(x) as usize;
    let tabs = TabRow::new(&ws.tabs, current, prompt);
    let pieces = right_pieces(input);
    let wanted = pieces.iter().map(|p| display_width(&p.text)).sum::<usize>() + 1;
    let right = wanted.min(room.saturating_sub(tabs.floor() + 1));
    let tabs_budget = room - right;
    tabs.draw(x, y, tabs_budget, buf);
    let mut cx = x + tabs_budget as u16;
    let last_x = right_edge.saturating_sub(1);
    for p in fit(pieces, right.saturating_sub(1)) {
        cx = put_within(buf, cx, y, last_x, &p.text, p.style.bg(bg));
    }
}

/// Fits the right end into `room` cells, cutting the piece the room runs out in and marking the
/// cut with `…`.
///
/// One cell of the room is the mark, and the mark is a piece of its own rather than part of the
/// cut piece's text. The cells can run out exactly on a piece boundary, and then nothing is cut
/// and `truncate_with_ellipsis` would hand back text it never touched - so the pieces after it
/// would be dropped with nothing to say so.
fn fit(pieces: Vec<Piece>, room: usize) -> Vec<Piece> {
    if pieces.iter().map(|p| display_width(&p.text)).sum::<usize>() <= room {
        return pieces;
    }
    if room == 0 {
        return Vec::new();
    }
    let mut left = room - 1;
    let mut out = Vec::new();
    // The mark continues the piece the room ran out in, so it takes that piece's style.
    let mut mark = Style::default().fg(theme::OVERLAY0);
    for p in pieces {
        let width = display_width(&p.text);
        mark = p.style;
        if width <= left {
            left -= width;
            out.push(p);
            continue;
        }
        if left > 0 {
            out.push(Piece {
                text: truncate_to_width(&p.text, left),
                style: p.style,
            });
        }
        break;
    }
    out.push(Piece {
        text: "…".into(),
        style: mark,
    });
    out
}

/// What the right end shows, in priority order: the prompt keys, the copy mode keys, the chord
/// indicator, a client hint, the config error, then the clock.
pub fn right_pieces(input: &RenderInput) -> Vec<Piece> {
    let key = Style::default().fg(theme::BLUE);
    let word = Style::default().fg(theme::OVERLAY0);
    let sep = Style::default().fg(theme::SURFACE1);
    let dot = || Piece {
        text: " · ".into(),
        style: sep,
    };
    if let Some(Overlay::Prompt(_)) = &input.view.overlay {
        return vec![
            Piece {
                text: "⏎".into(),
                style: key,
            },
            Piece {
                text: " save".into(),
                style: word,
            },
            dot(),
            Piece {
                text: "esc".into(),
                style: key,
            },
            Piece {
                text: " cancel".into(),
                style: word,
            },
            dot(),
            Piece {
                text: "empty clears".into(),
                style: word,
            },
        ];
    }
    if let Some(pieces) = crate::copy_mode::hint_pieces(input) {
        return pieces;
    }
    if let Some(chord) = &input.view.chord {
        let mut pieces = vec![Piece {
            text: chord.leader.clone(),
            style: key,
        }];
        // The help key as configured, not `?` (principle 3). It is a leader binding, so the
        // leader is stripped off the front: the indicator already shows it.
        if let Some(k) = input.keymap.key_for("help") {
            let after_leader = k
                .strip_prefix(&format!("{} ", chord.leader))
                .unwrap_or(k.as_str())
                .to_string();
            pieces.push(Piece {
                text: "  ".into(),
                style: word,
            });
            pieces.push(Piece {
                text: after_leader,
                style: key,
            });
            pieces.push(Piece {
                text: " keys".into(),
                style: word,
            });
        }
        return pieces;
    }
    if let Some(hint) = input.hint {
        return vec![Piece {
            text: hint.to_string(),
            style: word,
        }];
    }
    if let Some(err) = input.config_error {
        return vec![
            Piece {
                text: format!("domux.toml line {}: {}", err.line, err.message),
                style: Style::default().fg(theme::RED),
            },
            dot(),
            Piece {
                text: "domux2 config reload".into(),
                style: key,
            },
        ];
    }
    vec![Piece {
        text: input.now.format("%H:%M   %a %-d %b").to_string(),
        style: Style::default().fg(theme::SUBTEXT0),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(widths: &[usize]) -> Vec<Piece> {
        widths
            .iter()
            .map(|w| Piece {
                text: "x".repeat(*w),
                style: Style::default(),
            })
            .collect()
    }

    fn text(pieces: &[Piece]) -> String {
        pieces.iter().map(|p| p.text.as_str()).collect()
    }

    #[test]
    fn the_right_end_fits_untouched_when_the_room_is_enough() {
        assert_eq!(text(&fit(pieces(&[3, 5]), 8)), "xxxxxxxx");
        assert_eq!(text(&fit(pieces(&[3, 5]), 9)), "xxxxxxxx");
    }

    /// The cut that a piece-by-piece budget alone would let through silently: the room runs out
    /// exactly on a boundary, so no piece is ever truncated and the ellipsis never appears.
    #[test]
    fn the_right_end_marks_a_cut_that_falls_on_a_piece_boundary() {
        let out = fit(pieces(&[3, 5, 4]), 8);
        assert_eq!(text(&out), "xxxxxxx…");
        assert_eq!(display_width(&text(&out)), 8);
    }

    #[test]
    fn the_right_end_cuts_inside_the_piece_the_room_runs_out_in() {
        let out = fit(pieces(&[3, 9]), 8);
        assert_eq!(text(&out), "xxxxxxx…");
        let out = fit(pieces(&[12]), 4);
        assert_eq!(text(&out), "xxx…");
    }

    #[test]
    fn the_right_end_with_no_room_draws_nothing() {
        assert!(fit(pieces(&[9]), 0).is_empty());
        assert_eq!(text(&fit(pieces(&[9]), 1)), "…");
    }
}
