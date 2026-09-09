//! The Agents box's rows: `[dot] [session name or kind] [activity]` over
//! `[kind ·] project › workspace [› tab]`, then the recap in the agents overlay (interface
//! spec 6.2 and 6.3). One grammar on every surface, written once here: the sidebar drops the
//! tab and the recap, the agents overlay keeps both, and nothing else differs.
//!
//! Nothing here reads the Model. The core looks every field up once a frame and hands over an
//! `AgentsView`, so a row cannot show a place or a word the frame did not already resolve.

use crate::render::list_box::ListRow;
use crate::render::theme;
use chrono::{DateTime, Local};
use domux_core::ids::AgentId;
use domux_core::model::agent::{AgentKind, AgentState};
use domux_core::text::{display_width, truncate_with_ellipsis};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// The box's title, in the sidebar and in the agents overlay both (principle 14).
pub const TITLE: &str = "Agents";
/// Every row starts with one (interface spec 6.1: never empty of dots).
pub const DOT: &str = "●";
/// The recap's glyph.
pub const RECAP_GLYPH: &str = "※";
/// Two lines of recap, then an ellipsis (interface spec 12.20).
pub const RECAP_LINES: usize = 2;
/// Between the name and the activity on line 1 (interface spec 6.2).
const GAP: &str = "  ";
/// Between `exited 12 min ago` and the resume key (interface spec 6.2).
const RESUME_GAP: &str = "   ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowForm {
    /// Two lines: no tab, no recap.
    Sidebar,
    /// Three lines, recap included.
    Overlay,
}

/// One agent, with everything the row needs already looked up, so drawing touches no Model.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEntry {
    pub id: AgentId,
    pub kind: AgentKind,
    /// The session name the agent set. Absent until it does, and then the kind stands in.
    pub name: Option<String>,
    pub state: AgentState,
    pub unseen: bool,
    /// Absent when no recap arrived. An absent recap draws no line at all.
    pub recap: Option<String>,
    pub place_with_tab: String,
    pub place_without_tab: String,
    pub last_activity_at: String,
    /// The working word this agent holds, or `""` for a state that shows none. Never read
    /// outside `working`, so a stale word cannot reach a row that must not carry one.
    pub word: &'static str,
}

/// What the renderer reads. The core builds one per frame.
#[derive(Debug, Clone)]
pub struct AgentsView {
    /// In `Model::sorted_agents` order.
    pub agents: Vec<AgentEntry>,
    /// This frame's glyph (`labels::frame_at`).
    pub glyph: &'static str,
    pub now: DateTime<Local>,
    /// Agents that need you, across every project: the top bar's count (interface spec 6.8).
    pub red_dots: usize,
}

impl AgentsView {
    /// No agents. What a surface that draws none passes, and what a frame with an empty list
    /// holds.
    pub fn empty(now: DateTime<Local>) -> AgentsView {
        AgentsView {
            agents: Vec::new(),
            glyph: crate::agents::labels::frame_at(0),
            now,
            red_dots: 0,
        }
    }
}

/// The key a row carries and the cursor acts on.
pub fn row_key(id: &AgentId) -> String {
    id.to_string()
}

/// Every agent as one `ListRow`. `ListBox` puts the blank row between rows and scrolls them.
pub fn rows(view: &AgentsView, form: RowForm, width: u16) -> Vec<ListRow> {
    view.agents
        .iter()
        .map(|a| row(a, view, form, width))
        .collect()
}

fn row(a: &AgentEntry, view: &AgentsView, form: RowForm, width: u16) -> ListRow {
    let width = width as usize;
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(2 + RECAP_LINES);
    lines.push(Line::from(line_one(a, view, form, width)));
    lines.push(Line::from(line_two(a, form, width)));
    if form == RowForm::Overlay {
        if let Some(recap) = &a.recap {
            lines.extend(recap_lines(recap, a, width).into_iter().map(Line::from));
        }
    }
    ListRow::selectable(row_key(&a.id), filter_text(a), lines)
}

/// What `/` matches: the session name when there is one, the kind, and the place. Each field
/// can carry a match on its own, so `codex` finds every codex and `auth` finds the workspace
/// (interface spec 6.8). A filter that spans two adjacent fields matches by accident of this
/// line rather than by design, as it does in the Projects box.
fn filter_text(a: &AgentEntry) -> String {
    let mut out = String::new();
    if let Some(name) = &a.name {
        out.push_str(name);
        out.push(' ');
    }
    out.push_str(a.kind.as_str());
    out.push(' ');
    out.push_str(&a.place_with_tab);
    out
}

/// `[dot] [name] [activity]`, two spaces before the activity. The name is what gives way when
/// the row is too narrow: the activity says what the agent is doing and is short.
fn line_one(a: &AgentEntry, view: &AgentsView, form: RowForm, width: usize) -> Vec<Span<'static>> {
    let label = a
        .name
        .clone()
        .unwrap_or_else(|| a.kind.as_str().to_string());
    let activity = activity(a, view, form);
    let activity_width: usize = activity.iter().map(|s| display_width(&s.content)).sum();
    let lead = display_width(DOT) + 1;
    let gap = if activity.is_empty() {
        0
    } else {
        display_width(GAP)
    };
    let room = width.saturating_sub(lead + gap + activity_width);
    let mut spans = vec![
        Span::styled(DOT, Style::default().fg(dot_color(a))),
        Span::raw(" "),
        Span::styled(truncate_with_ellipsis(&label, room), label_style(a)),
    ];
    if !activity.is_empty() {
        spans.push(Span::raw(GAP));
        spans.extend(activity);
    }
    spans
}

/// The name in `text` bold, a kind standing in for one in the agent's colour, and both dimmed
/// on an exited or unknown row (interface spec 6.2).
fn label_style(a: &AgentEntry) -> Style {
    match a.state {
        AgentState::Exited | AgentState::Unknown => Style::default().fg(theme::OVERLAY0),
        _ if a.name.is_some() => Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(theme::agent_color(a.kind))
            .add_modifier(Modifier::BOLD),
    }
}

/// The activity, present only when it adds something (interface spec 6.2). A waiting or idle
/// row stops after the name, because "waiting" and "idle" are what the dot already says.
fn activity(a: &AgentEntry, view: &AgentsView, form: RowForm) -> Vec<Span<'static>> {
    match a.state {
        AgentState::Working => working(view.glyph, a.word, theme::agent_color(a.kind)),
        AgentState::Compacting => working(view.glyph, "Compacting", theme::COMPACTING),
        AgentState::Exited => {
            let ago = relative_time(&a.last_activity_at, view.now);
            // A timestamp that would not parse leaves the row saying `exited` and no more,
            // rather than an age nobody measured (principle 4).
            let since = if ago.is_empty() {
                "exited".to_string()
            } else {
                format!("exited {ago}")
            };
            let mut spans = vec![Span::styled(since, Style::default().fg(theme::OVERLAY1))];
            if form == RowForm::Overlay {
                // The sidebar has no room for the key; its hint row carries it while the
                // cursor is on the row (interface spec 12.6).
                spans.push(Span::raw(RESUME_GAP));
                spans.push(Span::styled("⏎ resume", Style::default().fg(theme::BLUE)));
            }
            spans
        }
        AgentState::Unknown => vec![Span::styled(
            "unknown",
            Style::default().fg(theme::OVERLAY0),
        )],
        AgentState::Waiting | AgentState::Idle => Vec::new(),
    }
}

/// `✶ Percolating…`: the frame's glyph, then the word and the ellipsis it is drawn with.
fn working(glyph: &'static str, word: &str, color: Color) -> Vec<Span<'static>> {
    let style = Style::default().fg(color);
    vec![
        Span::styled(glyph, style),
        Span::raw(" "),
        Span::styled(format!("{word}…"), style),
    ]
}

/// `[kind] · [place]`, or the place alone when line 1 already showed the kind.
fn line_two(a: &AgentEntry, form: RowForm, width: usize) -> Vec<Span<'static>> {
    let place = match form {
        RowForm::Sidebar => &a.place_without_tab,
        RowForm::Overlay => &a.place_with_tab,
    };
    let place_style = Style::default().fg(match form {
        RowForm::Overlay => theme::OVERLAY1,
        RowForm::Sidebar => theme::OVERLAY0,
    });
    if a.name.is_none() {
        return vec![Span::styled(
            truncate_with_ellipsis(place, width),
            place_style,
        )];
    }
    let kind = a.kind.as_str();
    let sep = " · ";
    let room = width.saturating_sub(display_width(kind) + display_width(sep));
    vec![
        Span::styled(kind, Style::default().fg(theme::agent_color(a.kind))),
        Span::styled(sep, Style::default().fg(theme::SURFACE1)),
        Span::styled(truncate_with_ellipsis(place, room), place_style),
    ]
}

/// `※ ` and the recap, wrapping onto a second line indented two spaces (interface spec 12.20).
/// A recap of nothing but spaces draws no line at all rather than an empty one.
fn recap_lines(recap: &str, a: &AgentEntry, width: usize) -> Vec<Vec<Span<'static>>> {
    // Both lines carry a two-cell lead: the glyph and its space, then the indent that lines
    // the wrap up under it.
    let indent = "  ";
    let room = width.saturating_sub(display_width(indent));
    let style = Style::default()
        .fg(recap_color(a))
        .add_modifier(Modifier::ITALIC);
    wrap(recap, room)
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let lead = if i == 0 {
                Span::styled(format!("{RECAP_GLYPH} "), style)
            } else {
                Span::raw(indent)
            };
            vec![lead, Span::styled(text, style)]
        })
        .collect()
}

/// Bright while the agent is working, waiting, compacting or unseen; `subtext0` once you have
/// seen it or it has exited (interface spec 6.2).
fn recap_color(a: &AgentEntry) -> Color {
    let live = matches!(
        a.state,
        AgentState::Working | AgentState::Waiting | AgentState::Compacting
    );
    if a.unseen || live {
        theme::RECAP
    } else {
        theme::RECAP_SEEN
    }
}

/// `text` broken on spaces into at most `RECAP_LINES` lines of `room` cells. The last line
/// takes everything that is left and ends in an ellipsis, so a long recap never stops
/// mid-sentence with no mark that it was cut.
fn wrap(text: &str, room: usize) -> Vec<String> {
    if room == 0 {
        return Vec::new();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut at = 0;
    while at < words.len() && lines.len() + 1 < RECAP_LINES {
        let mut line = String::new();
        while let Some(word) = words.get(at) {
            let candidate = if line.is_empty() {
                (*word).to_string()
            } else {
                format!("{line} {word}")
            };
            if display_width(&candidate) > room {
                break;
            }
            line = candidate;
            at += 1;
        }
        if line.is_empty() {
            // One word wider than the whole line. The last line takes it and cuts it, which
            // is the only thing that fits.
            break;
        }
        lines.push(line);
    }
    if at < words.len() {
        lines.push(truncate_with_ellipsis(&words[at..].join(" "), room));
    }
    lines
}

/// The dot's colour is the state, and unseen wins (interface spec 6.4 and 6.5).
fn dot_color(a: &AgentEntry) -> Color {
    // Unseen wins over the state, and a waiting row is red whether or not it is unseen.
    if a.unseen {
        return theme::RED;
    }
    match a.state {
        AgentState::Waiting => theme::RED,
        AgentState::Working => theme::agent_color(a.kind),
        AgentState::Compacting => theme::COMPACTING,
        AgentState::Idle | AgentState::Exited | AgentState::Unknown => theme::OVERLAY0,
    }
}

/// `12 min ago`, the way the artboard writes it. Both sides are instants, so a record stamped
/// in one offset and a clock reading in another still measure the same distance apart.
///
/// A timestamp that will not parse reads as nothing, never as `0 min ago` (principle 4).
pub fn relative_time(then: &str, now: DateTime<Local>) -> String {
    let Ok(then) = DateTime::parse_from_rfc3339(then) else {
        return String::new();
    };
    match now.timestamp() - then.timestamp() {
        // A stamp in the future falls here too, so a clock that runs ahead reads as
        // `just now` rather than a negative age.
        s if s < 60 => "just now".to_string(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::ids::AgentId;
    use domux_core::model::agent::{AgentKind, AgentState};
    use ratatui::style::Modifier;

    /// The frame's clock as an instant, not a wall-clock reading, so every row below reads
    /// the same whatever timezone the suite runs in.
    fn now() -> chrono::DateTime<chrono::Local> {
        chrono::DateTime::parse_from_rfc3339("2026-09-04T14:32:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Local)
    }

    /// One agent, with the place and recap the view would have looked up.
    fn entry(state: AgentState, name: Option<&str>, kind: AgentKind) -> AgentEntry {
        AgentEntry {
            id: AgentId("a_5e21".into()),
            kind,
            name: name.map(String::from),
            state,
            unseen: false,
            recap: Some(
                "Replaced three session checks with one guard in auth/middleware.go.".into(),
            ),
            place_with_tab: "audrey-app › auth cleanup › pr1".into(),
            place_without_tab: "audrey-app › auth cleanup".into(),
            last_activity_at: "2026-09-04T14:20:00+00:00".into(),
            word: "Percolating",
        }
    }

    fn view(entries: Vec<AgentEntry>) -> AgentsView {
        AgentsView {
            agents: entries,
            glyph: "✶",
            now: now(),
            red_dots: 0,
        }
    }

    /// The row's lines as plain text, for reading a failure.
    fn text(row: &ListRow) -> Vec<String> {
        row.lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_working_named_agent_reads_name_then_glyph_and_word_over_the_place_and_recap() {
        let v = view(vec![entry(
            AgentState::Working,
            Some("auth-cleanup"),
            AgentKind::Claude,
        )]);
        let rows = rows(&v, RowForm::Overlay, 72);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            text(&rows[0]),
            vec![
                "● auth-cleanup  ✶ Percolating…",
                "claude · audrey-app › auth cleanup › pr1",
                "※ Replaced three session checks with one guard in auth/middleware.go.",
            ]
        );
        let name = &rows[0].lines[0].spans[2];
        assert_eq!(name.content, "auth-cleanup");
        assert_eq!(name.style.fg, Some(theme::TEXT), "a name reads in text");
        assert!(name.style.add_modifier.contains(Modifier::BOLD), "and bold");
        let two = &rows[0].lines[1].spans;
        assert_eq!(
            two[0].style.fg,
            Some(theme::CLAUDE),
            "the kind's own colour"
        );
        assert_eq!(two[1].style.fg, Some(theme::SURFACE1), "the separator");
        assert_eq!(
            two[2].style.fg,
            Some(theme::OVERLAY1),
            "the place in the agents overlay"
        );
    }

    #[test]
    fn an_unnamed_agent_puts_the_kind_on_line_1_and_the_place_alone_on_line_2() {
        let v = view(vec![entry(AgentState::Working, None, AgentKind::Codex)]);
        let rows = rows(&v, RowForm::Overlay, 72);
        assert_eq!(text(&rows[0])[0], "● codex  ✶ Percolating…");
        assert_eq!(text(&rows[0])[1], "audrey-app › auth cleanup › pr1");
        let label = &rows[0].lines[0].spans[2];
        assert_eq!(label.content, "codex");
        assert_eq!(label.style.fg, Some(theme::CODEX), "a kind standing in");
        assert!(
            label.style.add_modifier.contains(Modifier::BOLD),
            "and bold"
        );
    }

    #[test]
    fn the_sidebar_form_drops_the_tab_and_the_recap() {
        let v = view(vec![entry(
            AgentState::Working,
            Some("auth-cleanup"),
            AgentKind::Claude,
        )]);
        let rows = rows(&v, RowForm::Sidebar, 36);
        assert_eq!(
            text(&rows[0]),
            vec![
                "● auth-cleanup  ✶ Percolating…",
                "claude · audrey-app › auth cleanup"
            ]
        );
        assert_eq!(
            rows[0].lines[1].spans[2].style.fg,
            Some(theme::OVERLAY0),
            "the sidebar's place is dimmer than the agents overlay's"
        );
    }

    #[test]
    fn waiting_and_idle_rows_carry_no_state_word() {
        for state in [AgentState::Waiting, AgentState::Idle] {
            let v = view(vec![entry(state, Some("auth-cleanup"), AgentKind::Claude)]);
            assert_eq!(
                text(&rows(&v, RowForm::Overlay, 72)[0])[0],
                "● auth-cleanup",
                "{state}"
            );
        }
    }

    #[test]
    fn compacting_reads_its_own_word_in_its_own_colour() {
        let v = view(vec![entry(
            AgentState::Compacting,
            Some("auth-cleanup"),
            AgentKind::Claude,
        )]);
        let rows = rows(&v, RowForm::Overlay, 72);
        assert_eq!(text(&rows[0])[0], "● auth-cleanup  ✶ Compacting…");
        let spans = &rows[0].lines[0].spans;
        assert_eq!(spans[0].style.fg, Some(theme::COMPACTING), "the dot");
        assert_eq!(
            spans.last().unwrap().style.fg,
            Some(theme::COMPACTING),
            "the word"
        );
    }

    #[test]
    fn an_exited_row_reads_how_long_ago_and_offers_resume() {
        let v = view(vec![entry(
            AgentState::Exited,
            Some("auth-cleanup"),
            AgentKind::Claude,
        )]);
        let overlay = rows(&v, RowForm::Overlay, 72);
        assert_eq!(
            text(&overlay[0])[0],
            "● auth-cleanup  exited 12 min ago   ⏎ resume"
        );
        let spans = &overlay[0].lines[0].spans;
        assert_eq!(
            spans.last().unwrap().style.fg,
            Some(theme::BLUE),
            "the key is blue"
        );
        // The sidebar drops the key; the hint row shows it while the cursor is on the row.
        assert_eq!(
            text(&rows(&v, RowForm::Sidebar, 36)[0])[0],
            "● auth-cleanup  exited 12 min ago"
        );
    }

    #[test]
    fn an_unknown_row_stands_the_kind_in_dimmed_and_says_unknown() {
        let mut e = entry(AgentState::Unknown, None, AgentKind::Claude);
        e.recap = None;
        let v = view(vec![e]);
        let rows = rows(&v, RowForm::Overlay, 72);
        assert_eq!(
            text(&rows[0]),
            vec!["● claude  unknown", "audrey-app › auth cleanup › pr1"]
        );
        assert_eq!(
            rows[0].lines[0].spans[0].style.fg,
            Some(theme::OVERLAY0),
            "a dim dot"
        );
        let label = &rows[0].lines[0].spans[2];
        assert_eq!(label.content, "claude");
        assert_eq!(
            label.style.fg,
            Some(theme::OVERLAY0),
            "the kind stands in dimmed, not in its own colour"
        );
    }

    #[test]
    fn the_dot_colour_is_the_state_and_unseen_wins() {
        let cases = [
            (AgentState::Working, false, theme::CLAUDE),
            (AgentState::Waiting, false, theme::RED),
            (AgentState::Idle, true, theme::RED),
            (AgentState::Idle, false, theme::OVERLAY0),
            (AgentState::Compacting, false, theme::COMPACTING),
            (AgentState::Exited, false, theme::OVERLAY0),
            (AgentState::Exited, true, theme::RED),
            (AgentState::Unknown, false, theme::OVERLAY0),
        ];
        for (state, unseen, colour) in cases {
            let mut e = entry(state, Some("x"), AgentKind::Claude);
            e.unseen = unseen;
            let v = view(vec![e]);
            let rows = rows(&v, RowForm::Overlay, 72);
            assert_eq!(rows[0].lines[0].spans[0].content, "●");
            assert_eq!(
                rows[0].lines[0].spans[0].style.fg,
                Some(colour),
                "{state} unseen={unseen}"
            );
        }
    }

    #[test]
    fn a_recap_wraps_onto_a_second_line_indented_two_spaces_and_stops_at_two() {
        let mut e = entry(AgentState::Idle, Some("auth-cleanup"), AgentKind::Claude);
        e.recap = Some("Replaced three session checks with one guard in auth middleware and then rewrote the token refresh path so the retry budget is shared across every caller of the client".into());
        let v = view(vec![e]);
        let rows = rows(&v, RowForm::Overlay, 60);
        let lines = text(&rows[0]);
        assert_eq!(lines.len(), 4, "name, place, recap, wrapped recap");
        assert!(lines[2].starts_with("※ "));
        assert!(
            lines[3].starts_with("  "),
            "the wrap is indented two spaces: {:?}",
            lines[3]
        );
        assert!(
            lines[3].ends_with('…'),
            "a recap longer than two lines ends in an ellipsis: {:?}",
            lines[3]
        );
        for l in &lines {
            assert!(
                domux_core::text::display_width(l) <= 60,
                "{l:?} is wider than the box"
            );
        }
    }

    #[test]
    fn a_recap_is_brighter_until_it_is_seen() {
        let mut e = entry(AgentState::Waiting, Some("x"), AgentKind::Claude);
        e.unseen = true;
        let bright = rows(&view(vec![e.clone()]), RowForm::Overlay, 72);
        assert_eq!(bright[0].lines[2].spans[1].style.fg, Some(theme::RECAP));
        assert!(
            bright[0].lines[2].spans[1]
                .style
                .add_modifier
                .contains(Modifier::ITALIC),
            "a recap is italic"
        );
        let mut seen = e.clone();
        seen.state = AgentState::Idle;
        seen.unseen = false;
        let dim = rows(&view(vec![seen]), RowForm::Overlay, 72);
        assert_eq!(dim[0].lines[2].spans[1].style.fg, Some(theme::RECAP_SEEN));
        // Unseen carries the brightness on its own, on a state that would not: an idle row
        // you have not looked at yet reads the same as a waiting one.
        let mut idle_unseen = e.clone();
        idle_unseen.state = AgentState::Idle;
        let bright_idle = rows(&view(vec![idle_unseen]), RowForm::Overlay, 72);
        assert_eq!(
            bright_idle[0].lines[2].spans[1].style.fg,
            Some(theme::RECAP)
        );
    }

    #[test]
    fn relative_time_reads_the_way_the_artboard_writes_it() {
        let n = now();
        assert_eq!(relative_time("2026-09-04T14:32:00+00:00", n), "just now");
        assert_eq!(relative_time("2026-09-04T14:31:10+00:00", n), "just now");
        assert_eq!(relative_time("2026-09-04T14:31:00+00:00", n), "1 min ago");
        assert_eq!(relative_time("2026-09-04T14:20:00+00:00", n), "12 min ago");
        assert_eq!(relative_time("2026-09-04T11:32:00+00:00", n), "3 h ago");
        assert_eq!(relative_time("2026-09-02T14:32:00+00:00", n), "2 d ago");
        assert_eq!(relative_time("not a timestamp", n), "");
        // A hook stamped by a clock that runs ahead reads as `just now`, not a negative age.
        assert_eq!(relative_time("2026-09-04T14:40:00+00:00", n), "just now");
    }

    #[test]
    fn a_long_name_and_place_are_cut_by_grapheme_and_never_overflow_the_sidebar() {
        let mut e = entry(
            AgentState::Working,
            Some("漢字 a very long session name 👍🏽 indeed"),
            AgentKind::Claude,
        );
        e.place_without_tab = "a-very-long-project-name › a very long workspace name".into();
        let rows = rows(&view(vec![e]), RowForm::Sidebar, 36);
        for l in text(&rows[0]) {
            assert!(domux_core::text::display_width(&l) <= 36, "{l:?}");
        }
    }

    #[test]
    fn every_row_starts_with_a_dot_and_carries_the_agents_id_as_its_key() {
        let v = view(vec![entry(AgentState::Idle, None, AgentKind::Opencode)]);
        let rows = rows(&v, RowForm::Overlay, 72);
        assert_eq!(rows[0].key.as_deref(), Some("a_5e21"));
        assert!(
            rows[0].filter_text.contains("opencode"),
            "the kind is what / matches when there is no name: {}",
            rows[0].filter_text
        );
        assert_eq!(rows[0].lines[0].spans[0].content, DOT);
    }
}
