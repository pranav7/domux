//! The rows of the Projects box (interface spec section 5), built once and placed twice: in
//! the sidebar and in the switcher. The switcher asks for two extra lines because it has the
//! width; nothing else differs, so the two surfaces cannot drift apart.

use crate::facts::FactRegistry;
use crate::render::list_box::{filter_rows, ListRow};
use crate::render::theme;
use domux_core::facts::{Fact, FactKey, FactState, FACT_BRANCH, FACT_PR};
use domux_core::model::{Model, Project, Workspace, WorkspaceHandle};
use domux_core::text::{display_width, truncate_with_ellipsis};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

pub const PROJECTS_TITLE: &str = "Projects";

/// Between the branch, the pull request number and the title on line 2.
const SEP: &str = " · ";

/// The fewest cells worth spending on a pull request title. Under this the title is dropped
/// whole, because a title cut to one syllable and an ellipsis says less than the room it
/// takes (interface spec 5.6).
const MIN_TITLE_WIDTH: usize = 8;

/// What this surface has room for. `width` is the box's inner width in cells.
#[derive(Debug, Clone, Copy)]
pub struct Extras {
    pub width: usize,
    /// The pull request's title after its number, and the tab list on its own line
    /// (interface spec 5.5). The sidebar says no; the switcher says yes.
    pub wide: bool,
}

impl Extras {
    pub fn compact(width: u16) -> Extras {
        Extras {
            width: width as usize,
            wide: false,
        }
    }

    pub fn switcher(width: u16) -> Extras {
        Extras {
            width: width as usize,
            wide: true,
        }
    }
}

/// Every row, in the order drawn: projects alphabetically (interface spec 12.15), `main`
/// first inside each and then the slots by number, one blank row between workspaces and one
/// before the next header. `filled` is the key of the row that carries the fill, which is
/// the cursor when focus is in the box and the current workspace otherwise (5.3).
///
/// `filter` is matched without case against the project name, the handle, the name, the
/// branch and the pull request number; a project whose workspaces all fail it disappears
/// with its header.
///
/// Every project holds `main`, so no header here is left without rows under it.
pub fn rows(
    model: &Model,
    facts: &FactRegistry,
    filter: &str,
    filled: Option<&str>,
    extras: Extras,
) -> Vec<ListRow> {
    let mut projects: Vec<&Project> = model.projects.iter().collect();
    projects.sort_by_key(|p| p.name.to_lowercase());
    let mut out: Vec<ListRow> = Vec::new();
    for project in projects {
        if !out.is_empty() {
            out.push(ListRow::blank());
        }
        out.push(header(&project.name, extras.width));
        for (i, w) in project.workspaces.iter().enumerate() {
            if i > 0 {
                out.push(ListRow::blank());
            }
            out.push(workspace_row(project, w, facts, filled, extras));
        }
    }
    // The box's own filter, not a second one here: `/` keeps the same rows in the sidebar,
    // in the switcher and in M3's Agents box because one function answers for all three.
    filter_rows(&out, filter)
}

/// `AUDREY-APP ─────────`: the name in upper case, one space, a rule to the box's edge.
/// A name too long for the box is shortened like any other (interface spec 5.6), and the
/// rule then has nothing left to draw.
fn header(name: &str, width: usize) -> ListRow {
    let label = truncate_with_ellipsis(&format!("{} ", name.to_uppercase()), width);
    let rule = "─".repeat(width.saturating_sub(display_width(&label)));
    ListRow::header(vec![Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(theme::OVERLAY1)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(rule, Style::default().fg(theme::SURFACE0)),
    ])])
}

fn workspace_row(
    project: &Project,
    w: &Workspace,
    facts: &FactRegistry,
    filled: Option<&str>,
    extras: Extras,
) -> ListRow {
    let branch = facts
        .get(&FactKey::workspace(&w.id, FACT_BRANCH))
        .map(|f| f.text.as_str());
    let pr = facts.get(&FactKey::workspace(&w.id, FACT_PR));
    let key = w.id.to_string();
    let filter_text = format!(
        "{} {} {} {} {}",
        project.name,
        w.handle,
        w.name.as_deref().unwrap_or_default(),
        branch.unwrap_or_default(),
        pr.map(|f| f.text.as_str()).unwrap_or_default(),
    );
    let mut lines = vec![line1(
        w,
        branch,
        pr.is_some(),
        filled == Some(key.as_str()),
        extras.width,
    )];
    if let Some(line) = line2(w, branch, pr, extras) {
        lines.push(line);
    }
    if extras.wide {
        if let Some(line) = tab_list(w) {
            lines.push(line);
        }
    }
    ListRow::selectable(key, filter_text, lines)
}

/// What line 1 says, which decides both its colour and how it brightens under the fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line1 {
    /// The word `main`.
    Main,
    /// `◌ workspace-2`: no name, no pull request, still on its own branch (interface spec
    /// 12.23).
    Untouched,
    /// A name, or the handle of a workspace that has a branch or a pull request, which
    /// interface spec 12.24 draws exactly as it draws a name.
    Live,
}

/// The name when there is one, else the handle. An untouched slot takes the hollow glyph
/// (interface spec 5.2).
fn line1(
    w: &Workspace,
    branch: Option<&str>,
    has_pr: bool,
    filled: bool,
    width: usize,
) -> Line<'static> {
    let (kind, text) = if w.is_untouched(branch, has_pr) {
        (Line1::Untouched, format!("◌ {}", w.handle))
    } else if w.name.is_none() && w.handle == WorkspaceHandle::Main {
        (Line1::Main, w.display_name())
    } else {
        (Line1::Live, w.display_name())
    };
    Line::from(Span::styled(
        truncate_with_ellipsis(&text, width),
        line1_style(kind, filled),
    ))
}

/// Interface spec 5.2 for the colours and 5.3 for the filled row.
///
/// 5.3 is split across two files, by design. `ListBox` draws the fill band and removes
/// `Modifier::DIM` from the filled line; it cannot do the rest, because removing the
/// dimming from an `overlay1` span leaves it `overlay1` rather than `text`, and the box
/// does not know which span is a name. So the bold and the `text` colour are decided here,
/// where the row knows what it is saying. Keep it here: a second rule for the fill in
/// `ListBox` would be a second answer to what a filled row looks like.
fn line1_style(kind: Line1, filled: bool) -> Style {
    let plain = Style::default().fg(match kind {
        Line1::Main => theme::OVERLAY1,
        Line1::Untouched => theme::OVERLAY0,
        Line1::Live => theme::TEAL,
    });
    if !filled {
        return plain;
    }
    match kind {
        // "A name goes bold." It keeps its teal, which is already bright, and a handle that
        // stands in for a name (12.24) brightens the same way the name it replaced would.
        Line1::Live => plain.add_modifier(Modifier::BOLD),
        // "`main` and a slot handle go from dim to `text`."
        Line1::Main | Line1::Untouched => plain.fg(theme::TEXT),
    }
}

/// `feat/auth-cleanup · PR#212 · Consolidate auth middleware`. The branch is dropped when it
/// would only repeat line 1: a slot on its own handle, or `main` on `main`. Truncation drops
/// the title, then shortens the branch, and never the number (interface spec 5.6 and 12.16).
fn line2(
    w: &Workspace,
    branch: Option<&str>,
    pr: Option<&Fact>,
    extras: Extras,
) -> Option<Line<'static>> {
    let branch = branch.filter(|b| !w.branch_is_handle(b));
    let pr_text = pr.map(|f| f.text.as_str());
    if branch.is_none() && pr_text.is_none() {
        return None;
    }
    let sep_width = display_width(SEP);
    let mut spans: Vec<Span<'static>> = Vec::new();
    if let Some(branch) = branch {
        // Everything the number and its separator do not need is the branch's.
        let room = extras.width.saturating_sub(
            pr_text.map(display_width).unwrap_or(0) + if pr_text.is_some() { sep_width } else { 0 },
        );
        // With no room at all the branch is gone, not an empty span: an empty span before
        // the separator would draw the line as ` · PR#212`, which reads as a branch whose
        // name is a space.
        let text = truncate_with_ellipsis(branch, room);
        if !text.is_empty() {
            spans.push(Span::styled(text, Style::default().fg(theme::PINK)));
        }
    }
    if let Some(pr_text) = pr_text {
        if !spans.is_empty() {
            spans.push(Span::styled(SEP, Style::default().fg(theme::SURFACE1)));
        }
        spans.push(Span::styled(
            pr_text.to_string(),
            pr_style(pr.and_then(|f| f.state.as_ref())),
        ));
    }
    if let Some(title) = pr.and_then(|f| f.url.as_deref()).filter(|_| extras.wide) {
        let used: usize = spans.iter().map(|s| display_width(&s.content)).sum();
        let room = extras.width.saturating_sub(used + sep_width);
        if room >= MIN_TITLE_WIDTH {
            spans.push(Span::styled(SEP, Style::default().fg(theme::SURFACE1)));
            spans.push(Span::styled(
                truncate_with_ellipsis(title, room),
                Style::default().fg(theme::OVERLAY1),
            ));
        }
    }
    Some(Line::from(spans))
}

/// `1 pr1     2 tests`: number, space, name, five spaces between tabs (interface spec 5.5).
/// Nothing here is shortened. Line 2 shortens itself because it has an order of importance
/// to express, the title before the branch; the tab list has none, so `ListBox` cuts what
/// does not fit and keeps each tab's own colour while it does.
fn tab_list(w: &Workspace) -> Option<Line<'static>> {
    if w.tabs.is_empty() {
        return None;
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, tab) in w.tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("     "));
        }
        spans.push(Span::styled(
            format!("{}", i + 1),
            Style::default().fg(theme::OVERLAY0),
        ));
        if let Some(name) = &tab.name {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                name.clone(),
                Style::default().fg(theme::OVERLAY1),
            ));
        }
    }
    Some(Line::from(spans))
}

/// V1's colours (`prStyleForState` in `picker.go`): green open, mauve merged, red closed,
/// grey draft. A state nobody reported takes the draft colour rather than a guess at open.
pub fn pr_style(state: Option<&FactState>) -> Style {
    let colour = match state {
        Some(FactState::Open) => theme::GREEN,
        Some(FactState::Merged) => theme::MAUVE,
        Some(FactState::Closed) => theme::RED,
        _ => theme::OVERLAY1,
    };
    Style::default().fg(colour)
}

/// The index of the row whose key is `key`, for `ListBox::filled`. Pass the key that built
/// the rows, or the band and the brightening land on different rows.
pub fn filled_index(rows: &[ListRow], key: Option<&str>) -> Option<usize> {
    let key = key?;
    rows.iter().position(|r| r.key.as_deref() == Some(key))
}

/// The key of the row at `index`, for `list.activate`. A header or a blank has none.
pub fn key_at(rows: &[ListRow], index: usize) -> Option<String> {
    rows.get(index).and_then(|r| r.key.clone())
}
