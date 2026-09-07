//! The full-width top bar: `project › workspace`, the tab row, `+`, and at the right end the
//! clock or whatever displaces it: prompt keys, copy mode keys, the leader indicator, a
//! client hint, the config error.

use crate::client::HintKind;
use crate::render::boxed::{put, put_within};
use crate::render::tab_row::TabRow;
use crate::render::{theme, RenderInput};
use domux_core::ids::TabId;
use domux_core::model::{ConfirmKind, Focus, Overlay};
use domux_core::text::{display_width, truncate_to_width};
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};

/// A run of text with one style, for the right end.
pub struct Piece {
    pub text: String,
    pub style: Style,
    /// The piece that gives up its cells first when the right end does not fit. At most one,
    /// and every piece after it keeps its cells whole: see `squeeze`.
    pub elastic: bool,
    /// A separator that exists only to join the elastic piece to the pieces after it. It has no
    /// meaning of its own, so when that piece is cut down to its mark this narrows to a space:
    /// ` · ` between a mark and an action reads as punctuation, not as elision.
    pub joiner: bool,
}

impl Piece {
    pub fn new(text: impl Into<String>, style: Style) -> Piece {
        Piece {
            text: text.into(),
            style,
            elastic: false,
            joiner: false,
        }
    }

    /// The separator between the elastic piece and what follows it. See `Piece::joiner`.
    pub fn joiner(text: impl Into<String>, style: Style) -> Piece {
        Piece {
            text: text.into(),
            style,
            elastic: false,
            joiner: true,
        }
    }

    /// A piece that is cut short so the pieces after it stay whole. The config error uses it:
    /// the message can be any length the file makes it, and the next action after it is the
    /// half of the notice the reader cannot act without (principle 9).
    pub fn elastic(text: impl Into<String>, style: Style) -> Piece {
        Piece {
            text: text.into(),
            style,
            elastic: true,
            joiner: false,
        }
    }
}

/// The cells an actionable right end keeps whatever the tab row wants: enough of the message
/// to read as a message, and the mark that says the rest was cut.
const RIGHT_FLOOR: usize = 8;

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
    // The room the tab row and the right end share, and how they share it.
    //
    // The last column is not part of it. An empty cell there keeps the bar reading as a bar
    // rather than as text pressed against the screen edge, so it comes off the top whichever of
    // the two would otherwise have reached it.
    //
    // Of what is left, the tab row keeps enough for the current tab, which has to be visible on
    // every frame (principle 2), and one more cell is the gap that keeps the right end off the
    // tab row - taken out of the tab row's drawing budget below, so it is a blank cell whichever
    // of the two wins the arithmetic here.
    // The right end takes what it wants from the rest and elides into it rather than running
    // off the edge (principle 6) - and unless what it shows is the clock, it keeps a floor of
    // its own even when that leaves the tab row less than its own: a message the reader has to
    // act on gives way to a mark, never to nothing (principle 9).
    let room = right_edge.saturating_sub(x).saturating_sub(1) as usize;
    // Whether the keys go to a pane rather than to a prompt or an overlay. It decides which run
    // of cells is accent-filled: see `tab_row::cell_for`.
    let pane_focus = matches!(input.view.focus, Focus::Pane(_));
    let tabs = TabRow::new(&ws.tabs, current, prompt, pane_focus);
    let end = right_end(input);
    let wanted: usize = end.pieces.iter().map(|p| display_width(&p.text)).sum();
    let floor = if end.actionable { RIGHT_FLOOR } else { 0 };
    let right = wanted
        .min(room.saturating_sub(tabs.floor() + 1))
        .max(floor.min(wanted).min(room));
    let tabs_budget = room - right;
    // One cell of the tab row's budget is the gap before the right end, left blank. The tab row
    // fills its budget to the last cell whenever it is cut, so without the gap its own cut mark
    // abuts the right end and the two read as one run of text - at the narrowest widths, two
    // elision marks running together (`indeed…… domux…`).
    let gap = usize::from(right > 0);
    tabs.draw(x, y, tabs_budget.saturating_sub(gap), buf);
    let last_x = right_edge.saturating_sub(2);
    let drawn = fit(squeeze(end.pieces, right), right);
    // Flush to the right, on the width the pieces actually came back with rather than on the
    // cells reserved for them: an elastic piece can shrink below its reservation, and a notice
    // that floats short of the edge reads as a label dropped mid-bar rather than as the end of
    // the bar.
    let width: usize = drawn.iter().map(|p| display_width(&p.text)).sum();
    let mut cx = x + room.saturating_sub(width) as u16;
    for p in drawn {
        cx = put_within(buf, cx, y, last_x, &p.text, p.style.bg(bg));
    }
}

/// Cuts the elastic piece short, if there is one, so the pieces after it keep their cells.
///
/// `fit` alone runs out of room from the left and drops whatever is still to come, which for the
/// config error is the `config reload` action - the one part of the notice the reader cannot act
/// without (principle 9). A message the file can make any length has to be the part that gives
/// way, not the answer to it.
///
/// The mark is written here rather than left to `fit`, because after this the pieces fit and
/// `fit` would hand them back untouched with nothing to say the message was cut.
fn squeeze(mut pieces: Vec<Piece>, room: usize) -> Vec<Piece> {
    let total: usize = pieces.iter().map(|p| display_width(&p.text)).sum();
    if total <= room {
        return pieces;
    }
    let Some(i) = pieces.iter().position(|p| p.elastic) else {
        return pieces;
    };
    let others = total - display_width(&pieces[i].text);
    // One cell of what is left over is the mark.
    let keep = room.saturating_sub(others).saturating_sub(1);
    if keep == 0 {
        // Not even one cell of its text survives. It becomes the mark rather than going: a
        // notice that shows only its next action reads as an offer rather than as an error,
        // and the mark keeps the piece's own style, so the alarm colour stays on the bar
        // (ruled 2026-09-07). Its joiner narrows to a space, because a right end that opens
        // with a bare ` · ` reads as a sentence with its subject cut off.
        pieces[i].text = "…".to_string();
        pieces[i].elastic = false;
        if let Some(j) = pieces.get_mut(i + 1).filter(|p| p.joiner) {
            j.text = " ".to_string();
        }
        return pieces;
    }
    pieces[i].text = format!("{}…", truncate_to_width(&pieces[i].text, keep));
    pieces
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
            out.push(Piece::new(truncate_to_width(&p.text, left), p.style));
        }
        break;
    }
    out.push(Piece::new("…", mark));
    out
}

/// What the right end shows, and whether the reader has to act on it.
pub struct RightEnd {
    pub pieces: Vec<Piece>,
    /// Everything but the clock. The clock is decorative: it is the one thing on the bar that
    /// gives way whole and silently when the room runs short. Anything else keeps `RIGHT_FLOOR`
    /// cells and elides into them, so nothing the reader has to act on leaves the screen with
    /// no mark to say it was there (ruled 2026-09-07).
    pub actionable: bool,
}

impl RightEnd {
    /// Live keys, or a state that is waiting for an answer.
    fn actionable(pieces: Vec<Piece>) -> RightEnd {
        RightEnd {
            pieces,
            actionable: true,
        }
    }

    /// The clock, and nothing else.
    fn decorative(pieces: Vec<Piece>) -> RightEnd {
        RightEnd {
            pieces,
            actionable: false,
        }
    }
}

/// What the right end shows, in priority order: the prompt keys, the chord indicator, the copy
/// mode keys, an action hint, the config error, a system hint, then the clock.
///
/// The chord indicator comes before the copy mode keys because it answers the key just pressed:
/// the leader inside copy mode used to start a chord the bar did not show, so the next key had
/// a meaning the screen had not admitted to (principle 8).
/// How the tab row labels this tab: its number, and its name when it has one. The question
/// has to name the tab the reader is looking at, and the number is the only part of a tab
/// that is always on screen.
fn tab_label(input: &RenderInput, tab: &TabId) -> String {
    let Some(ws) = input.model.workspace(&input.view.workspace) else {
        return "this tab".into();
    };
    let Some(i) = ws.tabs.iter().position(|t| &t.id == tab) else {
        return "this tab".into();
    };
    match &ws.tabs[i].name {
        Some(name) => format!("tab {} {}", i + 1, name),
        None => format!("tab {}", i + 1),
    }
}

pub fn right_end(input: &RenderInput) -> RightEnd {
    let key = Style::default().fg(theme::BLUE);
    let word = Style::default().fg(theme::OVERLAY0);
    let sep = Style::default().fg(theme::SURFACE1);
    let dot = || Piece::new(" · ", sep);
    // The config error's separator joins its message to the action after it, so it narrows with
    // that message rather than outliving it: see `Piece::joiner`.
    let joining_dot = || Piece::joiner(" · ", sep);
    // A question the reader has to answer outranks everything else the bar could say: until
    // they answer it, no other key does what it normally does.
    if let Some(Overlay::Confirm(ConfirmKind::CloseTab(tab))) = &input.view.overlay {
        return RightEnd::actionable(vec![
            Piece::new(format!("close {}?", tab_label(input, tab)), word),
            dot(),
            Piece::new("y", key),
            Piece::new(" close", word),
            dot(),
            Piece::new("esc", key),
            Piece::new(" keep", word),
        ]);
    }
    if let Some(Overlay::Prompt(_)) = &input.view.overlay {
        return RightEnd::actionable(vec![
            Piece::new("⏎", key),
            Piece::new(" save", word),
            dot(),
            Piece::new("esc", key),
            Piece::new(" cancel", word),
            dot(),
            Piece::new("empty clears", word),
        ]);
    }
    if let Some(chord) = &input.view.chord {
        let mut pieces = vec![Piece::new(chord.leader.clone(), key)];
        // The help key as configured, not `?` (principle 3). It is a leader binding, so the
        // leader is stripped off the front: the indicator already shows it.
        if let Some(k) = input.keymap.key_for("help") {
            let after_leader = k
                .strip_prefix(&format!("{} ", chord.leader))
                .unwrap_or(k.as_str())
                .to_string();
            pieces.push(Piece::new("  ", word));
            pieces.push(Piece::new(after_leader, key));
            pieces.push(Piece::new(" keys", word));
        }
        return RightEnd::actionable(pieces);
    }
    if let Some(pieces) = crate::copy_mode::hint_pieces(input) {
        return RightEnd::actionable(pieces);
    }
    // An action hint is the answer to the key just pressed and is gone on the next one, so it
    // outranks every state the bar was already showing (principle 8).
    if let Some(hint) = input.hint.filter(|h| h.kind == HintKind::Action) {
        return RightEnd::actionable(vec![Piece::new(hint.text.clone(), word)]);
    }
    // The config error before the shell-failure notice (ruled 2026-09-07). It is the newer of
    // the two - the config in force is still the old one, because the last edit was rejected -
    // and it is a prerequisite for the other: `terminal.shell` cannot be set until the file
    // parses at all.
    if let Some(err) = input.config_error {
        // `err.to_string()` names the file and, when one arrived, the line: the notice says
        // where to look rather than repeating a line number the error may not have.
        return RightEnd::actionable(vec![
            Piece::elastic(err.to_string(), Style::default().fg(theme::RED)),
            joining_dot(),
            Piece::new(
                format!("{} config reload", domux_core::names::BIN_NAME),
                key,
            ),
        ]);
    }
    if let Some(hint) = input.hint {
        return RightEnd::actionable(vec![Piece::new(hint.text.clone(), word)]);
    }
    RightEnd::decorative(vec![Piece::new(
        input.now.format("%H:%M   %a %-d %b").to_string(),
        Style::default().fg(theme::SUBTEXT0),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(widths: &[usize]) -> Vec<Piece> {
        widths
            .iter()
            .map(|w| Piece::new("x".repeat(*w), Style::default()))
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

    /// The config error's shape: a message that can be any length, then the next action. The
    /// message is what gives way, and the action arrives whole (principle 9).
    #[test]
    fn the_elastic_piece_gives_up_its_cells_so_the_pieces_after_it_stay_whole() {
        let notice = || {
            vec![
                Piece::elastic("domux.toml line 4: invalid string", Style::default()),
                Piece::new(" · ", Style::default()),
                Piece::new("domux2 config reload", Style::default()),
            ]
        };
        let out = fit(squeeze(notice(), 40), 40);
        assert_eq!(text(&out), "domux.toml line … · domux2 config reload");
        assert_eq!(display_width(&text(&out)), 40);
        // Room for everything: nothing is cut and no mark appears.
        let out = fit(squeeze(notice(), 60), 60);
        assert_eq!(
            text(&out),
            "domux.toml line 4: invalid string · domux2 config reload"
        );
    }

    /// Below the width the pieces after it need, there is nothing left to shorten the elastic
    /// piece to. It goes whole rather than leaving a lone `…`, and `fit` cuts what is left.
    #[test]
    fn an_elastic_piece_with_no_cells_left_for_it_becomes_its_mark_and_its_joiner_narrows() {
        let notice = || {
            vec![
                Piece::elastic("domux.toml line 4: invalid string", Style::default()),
                Piece::joiner(" · ", Style::default()),
                Piece::new("domux2 config reload", Style::default()),
            ]
        };
        // Not " · domux2 config re…": a right end that opens with a bare separator reads as a
        // sentence with its subject cut off, and one that shows only the action reads as an
        // offer rather than as an error.
        let out = fit(squeeze(notice(), 20), 20);
        assert_eq!(text(&out), "… domux2 config rel…");
        assert_eq!(display_width(&text(&out)), 20);
        // The floor an actionable right end keeps. Both marks survive it.
        let out = fit(squeeze(notice(), RIGHT_FLOOR), RIGHT_FLOOR);
        assert_eq!(text(&out), "… domux…");
        assert_eq!(display_width(&text(&out)), RIGHT_FLOOR);
    }

    /// The mark keeps the message's style, not the action's, so the alarm colour stays on the
    /// bar after the words it belonged to are gone.
    #[test]
    fn the_mark_left_by_an_elided_message_keeps_the_message_style() {
        let red = Style::default().fg(theme::RED);
        let notice = vec![
            Piece::elastic("domux.toml line 4: invalid string", red),
            Piece::joiner(" · ", Style::default()),
            Piece::new("domux2 config reload", Style::default()),
        ];
        let out = squeeze(notice, 20);
        assert_eq!(out[0].text, "…");
        assert_eq!(out[0].style, red);
        assert_eq!(out[1].text, " ");
    }

    /// Without an elastic piece the room still runs out from the left, so the right end that
    /// has no one part more important than another keeps behaving as it did.
    #[test]
    fn pieces_with_nothing_elastic_are_left_to_the_room_running_out() {
        assert_eq!(text(&fit(squeeze(pieces(&[3, 9]), 8), 8)), "xxxxxxx…");
    }

    #[test]
    fn the_right_end_with_no_room_draws_nothing() {
        assert!(fit(pieces(&[9]), 0).is_empty());
        assert_eq!(text(&fit(pieces(&[9]), 1)), "…");
    }
}
