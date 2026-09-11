//! The Agents box's rows: `[dot] [session name or kind] [activity]` over
//! `[kind ·] project › workspace [› tab]`, then the recap in the agents overlay (interface
//! spec 6.2 and 6.3). One grammar on every surface, written once here: the sidebar drops the
//! tab and the recap, the agents overlay keeps both, and nothing else differs.
//!
//! Nothing here reads the Model. The core looks every field up once a frame and hands over an
//! `AgentsView`, so a row cannot show a place or a word the frame did not already resolve.

use crate::render::list_box::ListRow;
use crate::render::projects_box::{self, INDENT};
use crate::render::theme;
use chrono::{DateTime, Local};
use domux_core::ids::{AgentId, WorkspaceId};
use domux_core::model::agent::{AgentKind, AgentState};
use domux_core::text::{display_width, truncate_with_ellipsis};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// The box's title, in the sidebar and in the agents overlay both (principle 14).
pub const TITLE: &str = "Agents";
/// The waiting mark. It is the only dot there is, and a row draws it only while its agent is
/// waiting on you (decision record 0030).
pub const DOT: &str = "●";
/// The recap's glyph.
pub const RECAP_GLYPH: &str = "※";
/// Two lines of recap, then an ellipsis (interface spec 12.20).
pub const RECAP_LINES: usize = 2;
/// Between the name and the activity on line 1 (interface spec 6.2).
const GAP: &str = "  ";

/// The arrow an agent row wears under its workspace in the Navigator. Two cells, like the
/// hollow glyph on an untouched slot, so every name in the box starts in one column.
pub const NEST: &str = "↳ ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowForm {
    /// The sidebar's Agents box: two lines, no tab, no recap. Retires with `[navigator]`.
    Sidebar,
    /// The agents overlay: three lines, recap included. Retires with `[navigator]`.
    Overlay,
    /// One line under its workspace in the Navigator's sidebar: the arrow, the name, the
    /// activity. The rows above it say the project and the workspace (decision record 0030).
    Nested,
    /// The same in the switcher, which has the width for the kind, the tab and the recap.
    NestedWide,
}

impl RowForm {
    /// Whether this form nests the row under a workspace rather than listing it flat.
    fn nested(self) -> bool {
        matches!(self, RowForm::Nested | RowForm::NestedWide)
    }
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
    /// The workspace the agent runs in, which is the row the Navigator nests it under.
    pub workspace: WorkspaceId,
    /// The project the record's workspace belongs to: the header the agents overlay groups
    /// under (MUX-21). Empty when the model no longer holds the workspace, and a group with an
    /// empty name is drawn with no header rather than a blank one.
    pub project: String,
    pub place_with_tab: String,
    pub place_without_tab: String,
    /// `workspace › tab`: what line 2 says under a project header, where the header has
    /// already said the project.
    pub place_in_project: String,
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
}

impl AgentsView {
    /// No agents. What a surface that draws none passes, and what a frame with an empty list
    /// holds. No row, so no key to name.
    pub fn empty(now: DateTime<Local>) -> AgentsView {
        AgentsView {
            agents: Vec::new(),
            glyph: crate::agents::labels::frame_at(0),
            now,
        }
    }
}

/// What a box with no rows to show says: the state, and the next action (principle 9).
///
/// Keyed on the filter and not on whether there are any records: a reader who has typed
/// something is being told about what they typed, and a reader who has not is being told how
/// to get a first agent. The sidebar's box and the agents overlay both call it, so an empty
/// list reads one way on both (plan assumption 22).
pub fn empty_text(filter: &str, form: RowForm) -> String {
    let filter = filter.trim();
    if !filter.is_empty() {
        return format!("No agent matches {filter:?}. esc clears the filter");
    }
    match form {
        // The Navigator's empty text is the Projects box's: an empty Navigator has no
        // projects in it, which is a bigger thing to say than having no agents.
        RowForm::Nested | RowForm::NestedWide | RowForm::Sidebar => {
            "Nothing running. Start claude or codex in a pane.".to_string()
        }
        RowForm::Overlay => "No agents yet. Start claude or codex in a pane.".to_string(),
    }
}

/// The key a row carries and the cursor acts on.
pub fn row_key(id: &AgentId) -> String {
    id.to_string()
}

/// Every agent as one `ListRow`, with one blank row between them (interface spec 6.2), and in
/// the agents overlay under a header per project (MUX-21).
///
/// The blanks are pushed here, the way `projects_box::rows` pushes its own. `ListBox` draws
/// each row's lines one after another and inserts nothing, and `filter_rows` drops these
/// blanks and rebuilds the same ones between the rows it keeps, so `/` changes what the list
/// holds and never its shape. `ListBox` owns the scrolling.
///
/// **Only the overlay groups.** The sidebar is 38 columns and its rows are already two lines
/// each; a header every few rows would spend the room the agents need, and the project is on
/// each row's own place line there. So the sidebar draws one flat list and the overlay draws
/// the same rows indented under headers, with `place_in_project` on line 2 because the header
/// above has said the project already.
///
/// **The projects come in the order their first agent does**, which is `sorted_agents` order,
/// so the project holding the agent that most wants you is the first group in the box. Sorting
/// the headers by name instead would put a waiting agent below two idle projects.
pub fn rows(view: &AgentsView, form: RowForm, width: u16) -> Vec<ListRow> {
    let mut out: Vec<ListRow> = Vec::with_capacity(view.agents.len().saturating_mul(2));
    let shown = || view.agents.iter();
    if form == RowForm::Sidebar {
        for a in shown() {
            if !out.is_empty() {
                out.push(ListRow::blank());
            }
            out.push(row(a, view, form, width));
        }
        return out;
    }
    let mut groups: Vec<&str> = Vec::new();
    for a in shown() {
        if !groups.contains(&a.project.as_str()) {
            groups.push(&a.project);
        }
    }
    for project in groups {
        // The grammar `projects_box::rows` writes and `filter_rows` rebuilds: a blank before
        // each header but the first, and none under it.
        if !out.is_empty() {
            out.push(ListRow::blank());
        }
        let indent = if project.is_empty() {
            // A record whose workspace the model no longer holds. There is no project to name,
            // so it takes no header and no indent rather than a blank one (principle 4).
            0
        } else {
            out.push(projects_box::header(project, width as usize));
            INDENT
        };
        let mut first = true;
        for a in shown().filter(|a| a.project == project) {
            if !first {
                out.push(ListRow::blank());
            }
            first = false;
            let row = row(a, view, form, width.saturating_sub(indent as u16));
            out.push(match indent {
                0 => row,
                _ => indented(row),
            });
        }
    }
    out
}

/// Every line of one row moved in by `INDENT`, the way `projects_box` insets a workspace under
/// its project. A raw span, so it carries no colour of its own and the fill's background is the
/// only thing it ever shows.
fn indented(row: ListRow) -> ListRow {
    let lines = row
        .lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ".repeat(INDENT))];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect();
    ListRow {
        lines,
        ..ListRow::selectable(row.key.unwrap_or_default(), row.filter_text, Vec::new())
    }
}

/// One agent's row, for a caller that places it itself. `render::projects_box` uses it to put
/// an agent under the workspace it runs in, so the Navigator draws the same grammar this box
/// draws and there is still one place it is written (principle 14).
pub fn one_row(a: &AgentEntry, view: &AgentsView, form: RowForm, width: u16) -> ListRow {
    row(a, view, form, width)
}

fn row(a: &AgentEntry, view: &AgentsView, form: RowForm, width: u16) -> ListRow {
    let width = width as usize;
    if form.nested() {
        return nested_row(a, view, form, width);
    }
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(2 + RECAP_LINES);
    lines.push(Line::from(line_one(a, view, width)));
    lines.push(Line::from(line_two(a, form, width)));
    if form == RowForm::Overlay {
        if let Some(recap) = &a.recap {
            lines.extend(recap_lines(recap, a, width).into_iter().map(Line::from));
        }
    }
    ListRow::selectable(row_key(&a.id), filter_text(a), lines)
}

/// One agent under the workspace it runs in: the arrow, then the name and the activity
/// (decision record 0030; artboard 9, frame 9.2 is the specification).
///
/// The place is not on it. The project is the header above and the workspace is the row above
/// that, so repeating either here is the reading MUX-21 complained of, one level deeper. What
/// the switcher adds is what the sidebar has no room for and the rows above never said: which
/// kind this is, which tab it is in, and what it did last.
fn nested_row(a: &AgentEntry, view: &AgentsView, form: RowForm, width: usize) -> ListRow {
    let lead = display_width(NEST);
    let room = width.saturating_sub(lead);
    let mut first = vec![Span::styled(NEST, Style::default().fg(theme::OVERLAY0))];
    let tail = if form == RowForm::NestedWide {
        kind_and_tab(a)
    } else {
        Vec::new()
    };
    let tail_width: usize = tail.iter().map(|s| display_width(&s.content)).sum();
    first.extend(line_one(a, view, room.saturating_sub(tail_width)));
    first.extend(tail);
    let mut lines = vec![Line::from(first)];
    if form == RowForm::NestedWide {
        if let Some(recap) = &a.recap {
            // The same two-cell lead the arrow takes, so the recap sits under the name.
            lines.extend(recap_lines(recap, a, room).into_iter().map(|spans| {
                let mut line = vec![Span::raw(" ".repeat(lead))];
                line.extend(spans);
                Line::from(line)
            }));
        }
    }
    ListRow::selectable(row_key(&a.id), filter_text(a), lines)
}

/// `  claude › pr1` after the activity, in the switcher only.
///
/// The kind is dropped when the row's label is already the kind, which is the rule `line_two`
/// follows for the same reason: an unnamed agent would otherwise read `codex  codex › pr2`.
/// The tab is dropped when the record names no pane the model still holds, which is what
/// `place_in_project` already says by leaving it off.
fn kind_and_tab(a: &AgentEntry) -> Vec<Span<'static>> {
    let tab = a.place_in_project.rsplit_once(" › ").map(|(_, t)| t);
    let mut spans = vec![Span::raw(GAP)];
    if a.name.is_some() {
        spans.push(Span::styled(
            a.kind.as_str(),
            Style::default().fg(theme::agent_color(a.kind)),
        ));
    }
    if let Some(tab) = tab {
        if a.name.is_some() {
            spans.push(Span::styled(" › ", Style::default().fg(theme::SURFACE1)));
        }
        spans.push(Span::styled(
            tab.to_string(),
            Style::default().fg(theme::OVERLAY1),
        ));
    }
    // Nothing to add, so not even the gap: a trailing pair of spaces would take two cells of
    // the name's budget for a field that is not there.
    if spans.len() == 1 {
        return Vec::new();
    }
    spans
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

/// `[name]  [activity]`, two spaces between them. The name is what gives way when the row is
/// too narrow: the activity says what the agent is doing and is short.
///
/// No leading dot. A dot is drawn only while the agent is waiting, and `activity` puts it in
/// the slot the working word would have taken, because a waiting agent draws no word and a
/// mark in front of the name would push that name out of the column every other row keeps it
/// in (decision record 0030).
fn line_one(a: &AgentEntry, view: &AgentsView, width: usize) -> Vec<Span<'static>> {
    let label = a
        .name
        .clone()
        .unwrap_or_else(|| a.kind.as_str().to_string());
    let activity = activity(a, view);
    let activity_width: usize = activity.iter().map(|s| display_width(&s.content)).sum();
    let gap = if activity.is_empty() {
        0
    } else {
        display_width(GAP)
    };
    let room = width.saturating_sub(gap + activity_width);
    let mut spans = vec![Span::styled(
        truncate_with_ellipsis(&label, room),
        label_style(a),
    )];
    if !activity.is_empty() {
        spans.push(Span::raw(GAP));
        spans.extend(activity);
    }
    spans
}

/// The name in `text` bold, a kind standing in for one in the agent's colour, and both dimmed
/// on an unknown row (interface spec 6.2).
fn label_style(a: &AgentEntry) -> Style {
    match a.state {
        AgentState::Unknown => Style::default().fg(theme::OVERLAY0),
        _ if a.name.is_some() => Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(theme::agent_color(a.kind))
            .add_modifier(Modifier::BOLD),
    }
}

/// What the row says after the name, and the only place a state is written down.
///
/// One slot, five answers. Working and compacting turn a glyph beside their word. Waiting is
/// the red dot, and it is the only dot in the box: the agent has asked you something and is
/// stopped until you answer. Idle says nothing, because nothing is happening and "idle" would
/// be a word for the absence of one (principle 5). Unknown says so, because an agent domux can
/// see and cannot hear is a fact worth reporting rather than a quiet row.
fn activity(a: &AgentEntry, view: &AgentsView) -> Vec<Span<'static>> {
    match a.state {
        AgentState::Working => working(view.glyph, a.word, theme::agent_color(a.kind)),
        AgentState::Compacting => working(view.glyph, "Compacting", theme::COMPACTING),
        AgentState::Waiting => vec![Span::styled(DOT, Style::default().fg(theme::RED))],
        AgentState::Unknown => vec![Span::styled(
            "unknown",
            Style::default().fg(theme::OVERLAY0),
        )],
        AgentState::Idle => Vec::new(),
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
///
/// The two flat forms only. A nested row's place is the rows above it (decision record 0030).
fn line_two(a: &AgentEntry, form: RowForm, width: usize) -> Vec<Span<'static>> {
    let place = match form {
        // The header above the row has already said the project (MUX-21).
        RowForm::Overlay => &a.place_in_project,
        _ => &a.place_without_tab,
    };
    let place_style = Style::default().fg(match form {
        RowForm::Overlay => theme::OVERLAY1,
        _ => theme::OVERLAY0,
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
/// seen it (interface spec 6.2).
fn recap_color(a: &AgentEntry) -> Color {
    let busy = matches!(
        a.state,
        AgentState::Working | AgentState::Waiting | AgentState::Compacting
    );
    if a.unseen || busy {
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
    use crate::render::list_box::filter_rows;
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
            workspace: WorkspaceId("w_c3a1".into()),
            project: "audrey-app".into(),
            place_with_tab: "audrey-app › auth cleanup › pr1".into(),
            place_without_tab: "audrey-app › auth cleanup".into(),
            place_in_project: "auth cleanup › pr1".into(),
            last_activity_at: "2026-09-04T14:20:00+00:00".into(),
            word: "Percolating",
        }
    }

    fn view(entries: Vec<AgentEntry>) -> AgentsView {
        AgentsView {
            agents: entries,
            glyph: "✶",
            now: now(),
        }
    }

    /// The agents overlay's rows that carry a key, built to `width` cells of text and with the
    /// group indent taken off.
    ///
    /// The grouping is one thing and the row grammar is another, and every test below but the
    /// two that name grouping is about the grammar. So the header and the indent are added by
    /// `rows` and taken off here, and a test that asks what a working row says reads the same
    /// string it read before MUX-21 put a project over it.
    fn overlay_rows(v: &AgentsView, width: u16) -> Vec<ListRow> {
        rows(v, RowForm::Overlay, width + INDENT as u16)
            .into_iter()
            .filter(|r| r.key.is_some())
            .map(|r| ListRow {
                lines: r
                    .lines
                    .into_iter()
                    .map(|line| {
                        let mut spans = line.spans;
                        if spans.first().is_some_and(|s| s.content == "  ") {
                            spans.remove(0);
                        }
                        Line::from(spans)
                    })
                    .collect(),
                ..ListRow::selectable(r.key.unwrap_or_default(), r.filter_text, Vec::new())
            })
            .collect()
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
        let rows = overlay_rows(&v, 72);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            text(&rows[0]),
            vec![
                "auth-cleanup  ✶ Percolating…",
                "claude · auth cleanup › pr1",
                "※ Replaced three session checks with one guard in auth/middleware.go.",
            ]
        );
        let name = &rows[0].lines[0].spans[0];
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
        let rows = overlay_rows(&v, 72);
        assert_eq!(text(&rows[0])[0], "codex  ✶ Percolating…");
        assert_eq!(
            text(&rows[0])[1],
            "auth cleanup › pr1",
            "the project is on the header, not on the row (MUX-21)"
        );
        let label = &rows[0].lines[0].spans[0];
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
                "auth-cleanup  ✶ Percolating…",
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
        for (state, line) in [
            (AgentState::Waiting, "auth-cleanup  ●"),
            (AgentState::Idle, "auth-cleanup"),
        ] {
            let v = view(vec![entry(state, Some("auth-cleanup"), AgentKind::Claude)]);
            assert_eq!(text(&overlay_rows(&v, 72)[0])[0], line, "{state}");
        }
    }

    #[test]
    fn compacting_reads_its_own_word_in_its_own_colour() {
        let v = view(vec![entry(
            AgentState::Compacting,
            Some("auth-cleanup"),
            AgentKind::Claude,
        )]);
        let rows = overlay_rows(&v, 72);
        assert_eq!(text(&rows[0])[0], "auth-cleanup  ✶ Compacting…");
        let spans = &rows[0].lines[0].spans;
        assert!(
            spans.iter().all(|s| s.content != DOT),
            "compacting draws no dot; the glyph and the word say it"
        );
        assert_eq!(
            spans.last().unwrap().style.fg,
            Some(theme::COMPACTING),
            "the word"
        );
    }

    #[test]
    fn an_unknown_row_stands_the_kind_in_dimmed_and_says_unknown() {
        let mut e = entry(AgentState::Unknown, None, AgentKind::Claude);
        e.recap = None;
        let v = view(vec![e]);
        let rows = overlay_rows(&v, 72);
        assert_eq!(
            text(&rows[0]),
            vec!["claude  unknown", "auth cleanup › pr1"]
        );
        let label = &rows[0].lines[0].spans[0];
        assert_eq!(label.content, "claude");
        assert_eq!(
            label.style.fg,
            Some(theme::OVERLAY0),
            "the kind stands in dimmed, not in its own colour"
        );
    }

    /// A dot is drawn only while an agent is waiting on you, and it is red (decision record
    /// 0030). Every other state draws none: working and compacting say themselves with the
    /// glyph and the word, idle has nothing to report, and unknown says so in a word.
    ///
    /// `unseen` is in the table twice because it used to win over the state here, which made
    /// the dot red on a record that had merely finished while you were looking elsewhere.
    #[test]
    fn a_dot_is_drawn_for_waiting_and_for_no_other_state() {
        let cases = [
            (AgentState::Waiting, false, true),
            (AgentState::Waiting, true, true),
            (AgentState::Working, false, false),
            (AgentState::Idle, true, false),
            (AgentState::Idle, false, false),
            (AgentState::Compacting, false, false),
            (AgentState::Unknown, false, false),
        ];
        for (state, unseen, dotted) in cases {
            let mut e = entry(state, Some("x"), AgentKind::Claude);
            e.unseen = unseen;
            let rows = overlay_rows(&view(vec![e]), 72);
            let spans = &rows[0].lines[0].spans;
            let dot = spans.iter().find(|s| s.content == DOT);
            assert_eq!(dot.is_some(), dotted, "{state} unseen={unseen}");
            if let Some(dot) = dot {
                assert_eq!(dot.style.fg, Some(theme::RED), "{state}");
                assert_ne!(
                    spans[0].content, DOT,
                    "the dot follows the name rather than leading the row"
                );
            }
        }
    }

    #[test]
    fn a_recap_wraps_onto_a_second_line_indented_two_spaces_and_stops_at_two() {
        let mut e = entry(AgentState::Idle, Some("auth-cleanup"), AgentKind::Claude);
        e.recap = Some("Replaced three session checks with one guard in auth middleware and then rewrote the token refresh path so the retry budget is shared across every caller of the client".into());
        let v = view(vec![e]);
        let rows = overlay_rows(&v, 60);
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
        let bright = overlay_rows(&view(vec![e.clone()]), 72);
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
        let dim = overlay_rows(&view(vec![seen]), 72);
        assert_eq!(dim[0].lines[2].spans[1].style.fg, Some(theme::RECAP_SEEN));
        // Unseen carries the brightness on its own, on a state that would not: an idle row
        // you have not looked at yet reads the same as a waiting one.
        let mut idle_unseen = e.clone();
        idle_unseen.state = AgentState::Idle;
        let bright_idle = overlay_rows(&view(vec![idle_unseen]), 72);
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
        // Both sides of the hour and of the day, so an off-by-one at either edge fails.
        assert_eq!(relative_time("2026-09-04T13:32:01+00:00", n), "59 min ago");
        assert_eq!(relative_time("2026-09-04T13:32:00+00:00", n), "1 h ago");
        assert_eq!(relative_time("2026-09-03T14:32:01+00:00", n), "23 h ago");
        assert_eq!(relative_time("2026-09-03T14:32:00+00:00", n), "1 d ago");
    }

    #[test]
    fn a_long_name_and_place_are_cut_by_grapheme_and_never_overflow_the_sidebar() {
        let mut e = entry(
            AgentState::Working,
            Some("漢字 a very long session name 👍🏽 indeed"),
            AgentKind::Claude,
        );
        e.place_without_tab = "a-very-long-project-name › a very long workspace name".into();
        // 34, which is `list_box::content_width` of the sidebar's 38: the border takes a
        // column each side and the box's pad takes another, and the drawing cuts anything
        // wider, so a row built to 36 would be cut by the drawing rather than laid out.
        let rows = rows(&view(vec![e]), RowForm::Sidebar, 34);
        for l in text(&rows[0]) {
            assert!(domux_core::text::display_width(&l) <= 34, "{l:?}");
        }
    }

    #[test]
    fn every_row_leads_with_its_name_and_carries_the_agents_id_as_its_key() {
        let v = view(vec![entry(AgentState::Idle, None, AgentKind::Opencode)]);
        let rows = overlay_rows(&v, 72);
        assert_eq!(rows[0].key.as_deref(), Some("a_5e21"));
        assert!(
            rows[0].filter_text.contains("opencode"),
            "the kind is what / matches when there is no name: {}",
            rows[0].filter_text
        );
        assert_eq!(
            rows[0].lines[0].spans[0].content, "opencode",
            "the row leads with the label, and an idle row draws nothing after it"
        );
    }

    #[test]
    fn two_agents_of_one_project_sit_under_one_header_with_a_blank_between_them() {
        let first = entry(AgentState::Waiting, Some("auth-cleanup"), AgentKind::Claude);
        let mut second = entry(AgentState::Idle, Some("billing-export"), AgentKind::Codex);
        second.id = AgentId("a_9c04".into());
        second.place_with_tab = "audrey-app › billing export › pr2".into();
        second.place_in_project = "billing export › pr2".into();
        let rows = rows(&view(vec![first, second]), RowForm::Overlay, 72);
        assert_eq!(rows.len(), 4, "a header, two agents and the blank between");
        assert!(
            rows[0].key.is_none(),
            "the header rests no cursor on itself"
        );
        assert!(text(&rows[0])[0].starts_with("AUDREY-APP "));
        assert_eq!(rows[1].key.as_deref(), Some("a_5e21"));
        assert_eq!(text(&rows[1])[0], "  auth-cleanup  ●", "indented under it");
        assert!(rows[2].is_blank(), "{:?}", text(&rows[2]));
        assert_eq!(rows[3].key.as_deref(), Some("a_9c04"));
        assert_eq!(text(&rows[3])[0], "  billing-export");
        // `/` changes what the list holds and never its shape: the filter drops these blanks
        // and rebuilds the same ones between the rows it keeps, header included.
        let both = filter_rows(&rows, "audrey-app");
        assert_eq!(both.len(), 4, "both agents still read the same way");
        assert_eq!(both[1].key.as_deref(), Some("a_5e21"));
        assert_eq!(both[3].key.as_deref(), Some("a_9c04"));
        let one = filter_rows(&rows, "billing");
        assert_eq!(
            one.len(),
            2,
            "one match keeps the header that names its place"
        );
        assert!(one[0].key.is_none());
        assert_eq!(one[1].key.as_deref(), Some("a_9c04"));
    }

    /// Two projects are two groups, and the group order is the row order: the project holding
    /// the agent that most wants you comes first, not the one whose name sorts first.
    #[test]
    fn each_project_is_a_group_and_the_groups_follow_the_row_order() {
        let mut waiting = entry(AgentState::Waiting, Some("auth-cleanup"), AgentKind::Claude);
        waiting.project = "zebra-app".into();
        waiting.place_in_project = "main › pr1".into();
        let mut idle = entry(AgentState::Idle, Some("billing-export"), AgentKind::Codex);
        idle.id = AgentId("a_9c04".into());
        idle.project = "audrey-app".into();
        idle.place_in_project = "main › pr2".into();
        let rows = rows(&view(vec![waiting, idle]), RowForm::Overlay, 72);
        assert_eq!(rows.len(), 5, "two headers, two agents, one blank between");
        assert!(text(&rows[0])[0].starts_with("ZEBRA-APP "));
        assert_eq!(rows[1].key.as_deref(), Some("a_5e21"));
        assert!(rows[2].is_blank());
        assert!(text(&rows[3])[0].starts_with("AUDREY-APP "));
        assert_eq!(rows[4].key.as_deref(), Some("a_9c04"));
    }

    /// The sidebar draws one flat list: 38 columns have no room for a header every few rows,
    /// and each row's own place line names the project there.
    #[test]
    fn the_sidebar_draws_no_headers_and_no_indent() {
        let mut second = entry(AgentState::Idle, Some("billing-export"), AgentKind::Codex);
        second.id = AgentId("a_9c04".into());
        let first = entry(AgentState::Waiting, Some("auth-cleanup"), AgentKind::Claude);
        let rows = rows(&view(vec![first, second]), RowForm::Sidebar, 34);
        assert_eq!(rows.len(), 3, "two agents and the blank between them");
        assert_eq!(text(&rows[0])[0], "auth-cleanup  ●");
        assert_eq!(text(&rows[0])[1], "claude · audrey-app › auth cleanup");
    }

    /// A record whose workspace the model no longer holds has no project to head it, so its
    /// rows take no header and no indent rather than an empty one (principle 4).
    #[test]
    fn a_record_with_no_project_takes_no_header() {
        let mut e = entry(AgentState::Idle, Some("orphan"), AgentKind::Claude);
        e.project = String::new();
        let rows = rows(&view(vec![e]), RowForm::Overlay, 72);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0])[0], "orphan");
    }

    #[test]
    fn a_recap_that_ends_on_the_second_line_carries_no_ellipsis() {
        let mut e = entry(AgentState::Idle, Some("auth-cleanup"), AgentKind::Claude);
        e.recap = Some(
            "Replaced three session checks with one guard in auth middleware and then \
             rewrote the token refresh"
                .into(),
        );
        let lines = text(&overlay_rows(&view(vec![e]), 60)[0]);
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[2],
            "※ Replaced three session checks with one guard in auth"
        );
        assert_eq!(lines[3], "  middleware and then rewrote the token refresh");
    }
}
