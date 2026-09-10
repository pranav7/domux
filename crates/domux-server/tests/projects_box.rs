use domux_core::facts::{Fact, FactKey, FactState, FACT_BRANCH, FACT_PR};
use domux_core::model::Model;
use domux_core::text::display_width;
use domux_server::facts::FactRegistry;
use domux_server::render::list_box::ListRow;
use domux_server::render::projects_box::{
    filled_index, key_at, pr_style, rows, Extras, PROJECTS_TITLE,
};
use domux_server::render::theme;
use ratatui::style::Modifier;
use std::path::PathBuf;
use std::time::Duration;

fn fact(text: &str, state: Option<FactState>) -> Fact {
    Fact::new(
        text,
        state,
        "2026-09-05T10:00:00+01:00",
        Duration::from_secs(600),
    )
}

/// A project with main, a named slot on a branch with a pull request, and an untouched slot.
fn model_and_facts() -> (Model, FactRegistry) {
    let mut m = Model::new(7);
    let (pid, main, _) = m
        .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
        .unwrap();
    let (w1, _) = m
        .add_slot(
            &pid,
            1,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1"),
        )
        .unwrap();
    let (w2, _) = m
        .add_slot(
            &pid,
            2,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-2"),
        )
        .unwrap();
    m.rename_workspace(&w1, Some("auth cleanup".into()))
        .unwrap();
    let mut f = FactRegistry::new();
    f.set(
        FactKey::workspace(&main, FACT_BRANCH),
        Some(fact("main", None)),
    );
    f.set(
        FactKey::workspace(&w1, FACT_BRANCH),
        Some(fact("feat/auth-cleanup", None)),
    );
    f.set(
        FactKey::workspace(&w1, FACT_PR),
        Some(fact("PR#212", Some(FactState::Open)).with_url("Consolidate auth middleware")),
    );
    f.set(
        FactKey::workspace(&w2, FACT_BRANCH),
        Some(fact("workspace-2", None)),
    );
    (m, f)
}

fn text(row: &ListRow, line: usize) -> String {
    row.lines[line]
        .spans
        .iter()
        .map(|s| s.content.to_string())
        .collect()
}

/// A workspace line with the two cells it leads with taken off: the indent, or the hollow
/// glyph of an untouched slot standing in it. Those two cells are pinned on their own below;
/// every other test here is about what the rest of the line says.
fn said(row: &ListRow, line: usize) -> String {
    let full = text(row, line);
    let lead: String = full.chars().take(2).collect();
    assert!(
        lead == "  " || lead == "\u{25cc} ",
        "a workspace line leads with the indent or the hollow glyph: {full:?}"
    );
    full.chars().skip(2).collect()
}

/// The id of the nth workspace of the first project, which is the key of its row.
fn key(m: &Model, project: usize, workspace: usize) -> String {
    m.projects[project].workspaces[workspace].id.to_string()
}

#[test]
fn the_rows_are_the_header_main_a_named_workspace_and_an_untouched_slot() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(
        out.len(),
        6,
        "header, main, blank, the named workspace, blank, the slot"
    );
    assert_eq!(
        text(&out[0], 0),
        "AUDREY-APP ─────────────────────────",
        "uppercase name, one space, a rule to the edge"
    );
    assert_eq!(out[0].key, None, "the cursor never rests on a header");
    assert_eq!(said(&out[1], 0), "main");
    assert_eq!(out[1].lines.len(), 1, "main on its own branch is one line");
    assert_eq!(text(&out[2], 0), "");
    assert!(
        out[2].is_blank(),
        "the two-line row below parts itself from the one above it"
    );
    assert_eq!(said(&out[3], 0), "auth cleanup");
    assert_eq!(said(&out[3], 1), "feat/auth-cleanup · PR#212");
    assert_eq!(out[3].lines.len(), 2, "the sidebar stops after line 2");
    assert_eq!(said(&out[5], 0), "workspace-2");
    assert_eq!(
        out[5].lines.len(),
        1,
        "an untouched slot drops the branch line that would repeat its handle"
    );
}

/// Every name in the box starts in the same column, and only the header sits at the edge.
///
/// The hollow glyph of an untouched slot hangs in the indent rather than standing in front of
/// the handle, so a marked row and an unmarked one line up.
#[test]
fn a_project_header_sits_at_the_edge_and_every_name_under_it_starts_two_cells_in() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert!(
        text(&out[0], 0).starts_with("AUDREY-APP"),
        "the header takes the box's own left edge: {:?}",
        text(&out[0], 0)
    );
    assert_eq!(text(&out[1], 0), "  main");
    assert_eq!(text(&out[3], 0), "  auth cleanup");
    assert_eq!(
        text(&out[3], 1),
        "  feat/auth-cleanup · PR#212",
        "line 2 is indented with line 1, so the row moves as one"
    );
    assert_eq!(
        text(&out[5], 0),
        "\u{25cc} workspace-2",
        "the glyph stands in the indent, so the handle starts where every name does"
    );
}

/// A named workspace draws its name, never the handle it stands in for.
///
/// `main` is the handle of the workspace at the project root, so a project whose main has
/// been named must not draw both. The row is the name and then the branch, and nothing else.
#[test]
fn a_named_workspace_draws_its_name_and_its_branch_and_no_handle() {
    let mut m = Model::new(7);
    let (_, main, _) = m
        .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
        .unwrap();
    m.rename_workspace(&main, Some("agent-harness".into()))
        .unwrap();
    let mut f = FactRegistry::new();
    f.set(
        FactKey::workspace(&main, FACT_BRANCH),
        Some(fact("p7/feat/harness", None)),
    );
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(
        out[1].lines.len(),
        2,
        "two lines, not three: {:?}",
        out[1]
            .lines
            .iter()
            .enumerate()
            .map(|(i, _)| text(&out[1], i))
            .collect::<Vec<_>>()
    );
    assert_eq!(said(&out[1], 0), "agent-harness");
    assert_eq!(said(&out[1], 1), "p7/feat/harness");
}

/// Blank rows separate projects and every join a two-line row is on, and nothing else.
///
/// The one-line case is the compact list the report asked for. The two-line case is what
/// stops a one-line row above it reading as its first line, which is how a whole workspace
/// goes missing from the reader's count.
#[test]
fn one_line_rows_stay_tight_and_a_two_line_row_is_parted_from_both_its_neighbours() {
    let mut m = Model::new(7);
    let (pid, _, _) = m
        .add_git_project(PathBuf::from("/repo/p"), "main".into())
        .unwrap();
    let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
    m.add_slot(&pid, 2, PathBuf::from("/w2")).unwrap();
    let mut f = FactRegistry::new();
    let tight = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(
        tight.len(),
        4,
        "the header and three workspaces of one line each, with nothing between them"
    );
    assert!(
        tight.iter().skip(1).all(|r| !r.is_blank()),
        "no blank row under the header"
    );

    // The middle workspace gains a branch line, so it is parted above as well as below.
    f.set(
        FactKey::workspace(&w1, FACT_BRANCH),
        Some(fact("feat/spike", None)),
    );
    let parted = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(parted.len(), 6, "two blanks added, and only two");
    assert_eq!(said(&parted[1], 0), "main");
    assert!(
        parted[2].is_blank(),
        "the one-line row above must not read as this row's first line"
    );
    assert_eq!(parted[3].lines.len(), 2);
    assert!(parted[4].is_blank(), "and the row below is parted too");
    assert_eq!(said(&parted[5], 0), "workspace-2");
}

#[test]
fn colours_follow_interface_spec_5_2_and_the_pull_request_state() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(
        out[0].lines[0].spans[0].style.fg,
        Some(theme::OVERLAY1),
        "the project name"
    );
    assert!(
        out[0].lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD),
        "the project name is bold"
    );
    assert_eq!(
        out[0].lines[0].spans[1].style.fg,
        Some(theme::SURFACE0),
        "the rule"
    );
    assert_eq!(
        out[1].lines[0].spans[0].style.fg,
        Some(theme::OVERLAY1),
        "main"
    );
    assert_eq!(
        out[3].lines[0].spans[0].style.fg,
        Some(theme::TEAL),
        "a workspace name"
    );
    // Span 0 of a line under line 1 is the indent, which carries no colour of its own.
    assert_eq!(
        out[3].lines[1].spans[1].style.fg,
        Some(theme::PINK),
        "the branch"
    );
    assert_eq!(
        out[3].lines[1].spans[2].style.fg,
        Some(theme::SURFACE1),
        "the middle dot"
    );
    assert_eq!(
        out[3].lines[1].spans[3].style.fg,
        Some(theme::GREEN),
        "an open pull request"
    );
    assert_eq!(
        out[5].lines[0].spans[0].style.fg,
        Some(theme::OVERLAY0),
        "an untouched slot"
    );
}

#[test]
fn every_pull_request_state_takes_v1_s_colour() {
    assert_eq!(pr_style(Some(&FactState::Open)).fg, Some(theme::GREEN));
    assert_eq!(pr_style(Some(&FactState::Merged)).fg, Some(theme::MAUVE));
    assert_eq!(pr_style(Some(&FactState::Closed)).fg, Some(theme::RED));
    assert_eq!(pr_style(Some(&FactState::Draft)).fg, Some(theme::OVERLAY1));
    assert_eq!(
        pr_style(Some(&FactState::Other("QUEUED".into()))).fg,
        Some(theme::OVERLAY1),
        "a state this version does not know is drawn like a draft"
    );
    assert_eq!(
        pr_style(None).fg,
        Some(theme::OVERLAY1),
        "a state nobody reported is drawn like a draft, not guessed as open"
    );
}

#[test]
fn an_unnamed_workspace_with_a_branch_shows_its_handle_in_teal() {
    let (m, mut f) = model_and_facts();
    let w2 = m.projects[0].workspaces[2].id.clone();
    f.set(
        FactKey::workspace(&w2, FACT_BRANCH),
        Some(fact("feat/spike", None)),
    );
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(
        text(&out[5], 0),
        "  workspace-2",
        "no hollow glyph once the slot has been touched, so the indent is plain"
    );
    assert_eq!(out[5].lines[0].spans[0].style.fg, Some(theme::TEAL));
    assert_eq!(said(&out[5], 1), "feat/spike");
}

#[test]
fn an_unnamed_workspace_on_its_own_branch_with_a_pull_request_is_not_untouched() {
    // The row that separates "no name and the branch is the handle" from "untouched": the
    // pull request alone makes the slot live, and its branch line is still dropped because
    // it would only repeat the handle.
    let (m, mut f) = model_and_facts();
    let w2 = m.projects[0].workspaces[2].id.clone();
    f.set(
        FactKey::workspace(&w2, FACT_PR),
        Some(fact("PR#9", Some(FactState::Merged))),
    );
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(text(&out[5], 0), "  workspace-2");
    assert_eq!(out[5].lines[0].spans[0].style.fg, Some(theme::TEAL));
    assert_eq!(
        said(&out[5], 1),
        "PR#9",
        "the number alone, with no separator in front of it"
    );
    assert_eq!(out[5].lines[1].spans[1].style.fg, Some(theme::MAUVE));
}

#[test]
fn a_workspace_whose_branch_never_arrived_draws_no_branch_line_and_no_guess() {
    let mut m = Model::new(7);
    let (pid, _, _) = m
        .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
        .unwrap();
    let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
    m.rename_workspace(&w1, Some("auth cleanup".into()))
        .unwrap();
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36)).rows;
    assert_eq!(said(&out[2], 0), "auth cleanup");
    assert_eq!(
        out[2].lines.len(),
        1,
        "an absent branch is absent, not the handle and not empty text"
    );
    assert_eq!(said(&out[1], 0), "main");
    assert_eq!(out[1].lines.len(), 1, "and the same for main");
}

#[test]
fn a_slot_with_no_branch_and_no_name_is_not_an_untouched_slot() {
    // `is_untouched` wants the branch to equal the handle. Absent is not equal, so the row
    // takes no hollow glyph until the branch has actually arrived (principle 4).
    let mut m = Model::new(7);
    let (pid, _, _) = m
        .add_git_project(PathBuf::from("/repo/p"), "main".into())
        .unwrap();
    m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36)).rows;
    assert_eq!(text(&out[2], 0), "  workspace-1");
    assert_eq!(out[2].lines[0].spans[0].style.fg, Some(theme::TEAL));
}

#[test]
fn main_on_a_branch_of_its_own_draws_the_branch_line_it_drops_on_main() {
    let mut m = Model::new(7);
    let (_, main, _) = m
        .add_git_project(PathBuf::from("/repo/p"), "main".into())
        .unwrap();
    let mut f = FactRegistry::new();
    f.set(
        FactKey::workspace(&main, FACT_BRANCH),
        Some(fact("feat/hotfix", None)),
    );
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(said(&out[1], 0), "main");
    assert_eq!(
        said(&out[1], 1),
        "feat/hotfix",
        "a branch that is not the handle is worth a line, on main as anywhere else"
    );
    assert_eq!(
        out[1].lines[0].spans[0].style.fg,
        Some(theme::OVERLAY1),
        "main keeps its colour on any branch"
    );
}

#[test]
fn a_named_main_draws_its_name_and_not_the_handle() {
    let mut m = Model::new(7);
    let (_, main, _) = m
        .add_git_project(PathBuf::from("/repo/p"), "main".into())
        .unwrap();
    m.rename_workspace(&main, Some("trunk".into())).unwrap();
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36)).rows;
    assert_eq!(said(&out[1], 0), "trunk");
    assert_eq!(
        out[1].lines[0].spans[0].style.fg,
        Some(theme::TEAL),
        "a name is a name, on main as on a slot"
    );
}

#[test]
fn projects_are_listed_alphabetically_whatever_order_they_were_added_in() {
    // The fixture has to separate four sorts, not one. `Zeta` before `alpha` in byte order
    // tells a letter sort from a byte one; `/repo/a/Zeta` before `/repo/b/alpha` in path
    // order tells a sort on the name from a sort on the directory it came from; and three
    // projects rather than two, on seed 4, put the ids in the order `Zeta`, `alpha`, `Mid`,
    // which no correct sort produces. `Model::new` seeds the id generator, so the fixture is
    // the same on every run.
    let mut m = Model::new(4);
    m.add_git_project(PathBuf::from("/repo/a/Zeta"), "main".into())
        .unwrap();
    m.add_git_project(PathBuf::from("/repo/b/alpha"), "main".into())
        .unwrap();
    m.add_git_project(PathBuf::from("/repo/c/Mid"), "main".into())
        .unwrap();
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36)).rows;
    let headers: Vec<String> = [0usize, 3, 6]
        .iter()
        .map(|i| {
            text(&out[*i], 0)
                .split(' ')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(headers, vec!["ALPHA", "MID", "ZETA"]);
    assert!(out[2].is_blank(), "one blank row before the next header");
    assert!(out[5].is_blank(), "and before the one after that");
}

#[test]
fn a_project_name_too_wide_for_the_box_is_shortened_and_leaves_no_rule() {
    let mut m = Model::new(7);
    m.add_git_project(
        PathBuf::from("/repo/a-project-with-a-very-long-name-indeed"),
        "main".into(),
    )
    .unwrap();
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(20)).rows;
    assert_eq!(text(&out[0], 0), "A-PROJECT-WITH-A-VE…");
    assert_eq!(
        out[0].lines[0].spans[1].content, "",
        "the rule has nothing left to draw"
    );
}

#[test]
fn the_switcher_adds_the_pull_request_title_when_the_width_allows() {
    let (mut m, f) = model_and_facts();
    let w1 = m.projects[0].workspaces[1].id.clone();
    let (t1, _, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
    m.rename_tab(&t1, Some("pr1".into())).unwrap();
    m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
    let out = rows(&m, &f, "", None, Extras::switcher(60)).rows;
    assert_eq!(
        said(&out[3], 1),
        "feat/auth-cleanup · PR#212 · Consolidate auth middleware"
    );
    assert_eq!(
        out[3].lines[1].spans[4].style.fg,
        Some(theme::SURFACE1),
        "the middle dot before the title"
    );
    assert_eq!(
        out[3].lines[1].spans[5].style.fg,
        Some(theme::OVERLAY1),
        "the title"
    );
    let narrow = rows(&m, &f, "", None, Extras::switcher(32)).rows;
    assert_eq!(
        said(&narrow[3], 1),
        "feat/auth-cleanup · PR#212",
        "the title goes first when the column is tight"
    );
}

/// MUX-12: a row is its name and its branch on both surfaces, and never the tabs under it.
///
/// The two tabs are named and the box is 80 cells wide, so nothing here is short of room:
/// what the switcher leaves out, it leaves out because the line said nothing worth a row.
#[test]
fn a_workspace_row_is_two_lines_at_most_on_either_surface() {
    let (mut m, f) = model_and_facts();
    let w1 = m.projects[0].workspaces[1].id.clone();
    let (t1, _, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
    m.rename_tab(&t1, Some("pr1".into())).unwrap();
    let (t2, _, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
    m.rename_tab(&t2, Some("tests".into())).unwrap();

    let sidebar = rows(&m, &f, "", None, Extras::compact(80)).rows;
    assert_eq!(
        said(&sidebar[3], 1),
        "feat/auth-cleanup · PR#212",
        "no title in the sidebar, however wide the box happens to be"
    );
    assert_eq!(sidebar[3].lines.len(), 2, "the name and the branch");
    let switcher = rows(&m, &f, "", None, Extras::switcher(80)).rows;
    assert_eq!(switcher[3].lines.len(), 2, "the same two in the switcher");
}

#[test]
fn truncation_drops_the_title_then_shortens_the_branch_and_never_the_pull_request_number() {
    let mut m = Model::new(7);
    let (pid, _, _) = m
        .add_git_project(PathBuf::from("/repo/p"), "main".into())
        .unwrap();
    let (w, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
    m.rename_workspace(&w, Some("a very long workspace name indeed".into()))
        .unwrap();
    let mut f = FactRegistry::new();
    f.set(
        FactKey::workspace(&w, FACT_BRANCH),
        Some(fact("feat/a-very-long-branch-name-here", None)),
    );
    f.set(
        FactKey::workspace(&w, FACT_PR),
        Some(fact("PR#212", Some(FactState::Open)).with_url("A long title")),
    );
    let out = rows(&m, &f, "", None, Extras::switcher(28)).rows;
    let line2 = said(&out[3], 1);
    // 26 cells left of the box's 28 once the indent has its two: the branch keeps what the
    // number (6) and the separator (3) leave it.
    assert_eq!(line2, "feat/a-very-long… · PR#212");
    assert!(
        line2.ends_with("PR#212"),
        "the number is never cut (interface spec 12.16)"
    );
    assert_eq!(said(&out[3], 0), "a very long workspace nam…");
}

/// A workspace with a short branch, a number and a long title, so the title is the only
/// thing the width can shorten.
fn model_with_a_title() -> (Model, FactRegistry) {
    let mut m = Model::new(7);
    let (pid, _, _) = m
        .add_git_project(PathBuf::from("/repo/p"), "main".into())
        .unwrap();
    let (w, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
    m.rename_workspace(&w, Some("auth".into())).unwrap();
    let mut f = FactRegistry::new();
    f.set(
        FactKey::workspace(&w, FACT_BRANCH),
        Some(fact("feat/x", None)),
    );
    f.set(
        FactKey::workspace(&w, FACT_PR),
        Some(fact("PR#212", Some(FactState::Open)).with_url("Consolidate auth middleware")),
    );
    (m, f)
}

#[test]
fn a_title_too_long_for_the_line_is_shortened_to_what_is_left_of_it() {
    let (m, f) = model_with_a_title();
    // `feat/x · PR#212` is 15 cells and the title's own separator is 3, so at 42 the indent
    // takes two, the title gets the remaining 22 and the line fills the box exactly.
    let out = rows(&m, &f, "", None, Extras::switcher(42)).rows;
    let line = said(&out[3], 1);
    assert_eq!(line, "feat/x · PR#212 · Consolidate auth midd…");
    assert_eq!(
        display_width(&text(&out[3], 1)),
        42,
        "the line never runs past the box, so the box has nothing to cut"
    );
}

#[test]
fn a_title_with_under_eight_cells_to_live_in_is_dropped_rather_than_shortened() {
    let (m, f) = model_with_a_title();
    // 15 for `feat/x · PR#212`, 3 for the separator, 2 for the indent: at 28 the title has 8
    // cells, at 27 it has 7 and is not worth its separator.
    let drawn = rows(&m, &f, "", None, Extras::switcher(28)).rows;
    assert_eq!(said(&drawn[3], 1), "feat/x · PR#212 · Consoli…");
    let dropped = rows(&m, &f, "", None, Extras::switcher(27)).rows;
    assert_eq!(said(&dropped[3], 1), "feat/x · PR#212");
}

#[test]
fn a_branch_the_pull_request_number_leaves_no_room_for_takes_no_separator_with_it() {
    let mut m = Model::new(7);
    let (pid, _, _) = m
        .add_git_project(PathBuf::from("/repo/p"), "main".into())
        .unwrap();
    let (w, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
    let mut f = FactRegistry::new();
    f.set(
        FactKey::workspace(&w, FACT_BRANCH),
        Some(fact("feat/spike", None)),
    );
    f.set(
        FactKey::workspace(&w, FACT_PR),
        Some(fact("PR#212", Some(FactState::Open))),
    );
    let out = rows(&m, &f, "", None, Extras::compact(11)).rows;
    assert_eq!(
        said(&out[3], 1),
        "PR#212",
        "the number alone, with no leading separator where the branch used to be"
    );
    assert_eq!(
        out[3].lines[1].spans.len(),
        2,
        "the indent and the number, and nothing between them"
    );
}

#[test]
fn the_filter_keeps_matching_workspaces_with_their_header_and_says_so_when_nothing_matches() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "auth", None, Extras::compact(36)).rows;
    assert_eq!(out.len(), 2, "the header and the one match");
    assert_eq!(said(&out[1], 0), "auth cleanup");
    let branch = rows(&m, &f, "FEAT/", None, Extras::compact(36)).rows;
    assert_eq!(
        branch.len(),
        2,
        "the filter matches the branch too, without case"
    );
    assert!(
        rows(&m, &f, "zzz", None, Extras::compact(36))
            .rows
            .is_empty(),
        "no matches means no rows; the box draws its empty text"
    );
}

#[test]
fn each_field_of_the_filter_text_can_carry_a_match_on_its_own() {
    // Five fields go into a row's filter text, so five filters, each of which only one of
    // them can answer. A fixture where the branch happens to repeat the handle, or where the
    // branch contains the name, hides a field that has stopped being matched at all.
    let mut m = Model::new(7);
    let (pid, _, _) = m
        .add_git_project(PathBuf::from("/repo/notes-app"), "main".into())
        .unwrap();
    let (w1, _) = m.add_slot(&pid, 1, PathBuf::from("/w1")).unwrap();
    m.add_slot(&pid, 2, PathBuf::from("/w2")).unwrap();
    m.rename_workspace(&w1, Some("auth cleanup".into()))
        .unwrap();
    let mut f = FactRegistry::new();
    f.set(
        FactKey::workspace(&w1, FACT_BRANCH),
        Some(fact("feat/xyz-9", None)),
    );
    f.set(
        FactKey::workspace(&w1, FACT_PR),
        Some(fact("PR#77", Some(FactState::Open))),
    );

    let by_name = rows(&m, &f, "cleanup", None, Extras::compact(36)).rows;
    assert_eq!(by_name.len(), 2, "the header and the named workspace");
    assert_eq!(
        said(&by_name[1], 0),
        "auth cleanup",
        "the name, which no branch here repeats"
    );

    let by_handle = rows(&m, &f, "workspace-2", None, Extras::compact(36)).rows;
    assert_eq!(by_handle.len(), 2, "the header and the slot");
    assert_eq!(
        said(&by_handle[1], 0),
        "workspace-2",
        "the handle, which no branch fact here repeats"
    );

    let by_branch = rows(&m, &f, "XYZ", None, Extras::compact(36)).rows;
    assert_eq!(by_branch.len(), 2, "the branch, matched without case");
    assert_eq!(said(&by_branch[1], 0), "auth cleanup");

    let by_number = rows(&m, &f, "pr#77", None, Extras::compact(36)).rows;
    assert_eq!(by_number.len(), 2, "the pull request number");
    assert_eq!(said(&by_number[1], 0), "auth cleanup");

    assert!(
        rows(&m, &f, "notes-appmain", None, Extras::compact(36))
            .rows
            .is_empty(),
        "the fields stay apart, so no filter matches across the join between two of them"
    );

    let by_project = rows(&m, &f, "notes-app", None, Extras::compact(36)).rows;
    assert_eq!(
        by_project.len(),
        6,
        "a project name keeps every workspace under it, blanks and all"
    );
    assert_eq!(
        vec![
            said(&by_project[1], 0),
            said(&by_project[3], 0),
            said(&by_project[5], 0)
        ],
        vec!["main", "auth cleanup", "workspace-2"],
    );
}

#[test]
fn the_filter_keeps_the_blank_row_between_two_matches() {
    // Interface spec 5.2's grammar does not change under the filter: `/` chooses which rows
    // are drawn, not how they are separated.
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "audrey", None, Extras::compact(36)).rows;
    assert_eq!(out.len(), 6, "the header and all three workspaces");
    assert_eq!(
        vec![said(&out[1], 0), said(&out[3], 0), said(&out[5], 0)],
        vec!["main", "auth cleanup", "workspace-2"],
    );
    assert!(out[2].is_blank(), "one blank between two matches");
    assert!(out[4].is_blank(), "and between the next two");
}

#[test]
fn the_filled_row_takes_the_bold_and_the_bright_text_the_box_cannot_give_it() {
    // Interface spec 5.3 is drawn by two files: `ListBox` lays the band and removes the
    // dimming, and the row builder decides the bold and the `text` colour. This pins the
    // half that lives here, and that the band is not built here.
    let (m, f) = model_and_facts();
    let built = rows(&m, &f, "", Some(&key(&m, 0, 1)), Extras::compact(36));
    assert_eq!(
        built.filled,
        Some(3),
        "the index comes back with the rows, from the key that styled them"
    );
    let named = built.rows;
    let style = named[3].lines[0].spans[0].style;
    assert_eq!(style.fg, Some(theme::TEAL), "a name keeps its teal");
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "a name goes bold"
    );
    assert_eq!(style.bg, None, "the band is the box's, not the row's");
    assert!(
        !named[3].lines[1].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD),
        "the fill is on line 1 only"
    );
    assert_eq!(
        named[1].lines[0].spans[0].style.fg,
        Some(theme::OVERLAY1),
        "no other row changes"
    );

    let on_main = rows(&m, &f, "", Some(&key(&m, 0, 0)), Extras::compact(36)).rows;
    let style = on_main[1].lines[0].spans[0].style;
    assert_eq!(style.fg, Some(theme::TEXT), "main goes from dim to text");
    assert!(
        !style.add_modifier.contains(Modifier::BOLD),
        "main brightens without going bold"
    );
    assert_eq!(
        on_main[3].lines[0].spans[0].style.fg,
        Some(theme::TEAL),
        "and the name that is not filled stays as it was"
    );

    let on_slot = rows(&m, &f, "", Some(&key(&m, 0, 2)), Extras::compact(36)).rows;
    let style = on_slot[5].lines[0].spans[0].style;
    assert_eq!(
        style.fg,
        Some(theme::TEXT),
        "a slot handle goes from dim to text"
    );
    assert!(!style.add_modifier.contains(Modifier::BOLD));

    let none = rows(&m, &f, "", None, Extras::compact(36)).rows;
    assert_eq!(none[1].lines[0].spans[0].style.fg, Some(theme::OVERLAY1));
    assert_eq!(none[5].lines[0].spans[0].style.fg, Some(theme::OVERLAY0));
    assert!(!none[3].lines[0].spans[0]
        .style
        .add_modifier
        .contains(Modifier::BOLD));
}

#[test]
fn a_key_that_names_no_row_fills_no_row() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", Some("w_ffff"), Extras::compact(36)).rows;
    assert_eq!(out[3].lines[0].spans[0].style.fg, Some(theme::TEAL));
    assert!(!out[3].lines[0].spans[0]
        .style
        .add_modifier
        .contains(Modifier::BOLD));
    assert_eq!(filled_index(&out, Some("w_ffff")), None);
}

#[test]
fn a_filled_row_the_filter_dropped_fills_nothing() {
    let (m, f) = model_and_facts();
    let built = rows(
        &m,
        &f,
        "workspace-2",
        Some(&key(&m, 0, 1)),
        Extras::compact(36),
    );
    assert_eq!(text(&built.rows[1], 0), "◌ workspace-2", "the one match");
    assert_eq!(
        built.filled, None,
        "the row the key named is not in the list, so nothing carries the fill"
    );
}

#[test]
fn filled_index_and_key_at_name_the_same_row() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::compact(36)).rows;
    let w1 = key(&m, 0, 1);
    assert_eq!(filled_index(&out, Some(&w1)), Some(3));
    assert_eq!(key_at(&out, 3).as_deref(), Some(w1.as_str()));
    assert_eq!(filled_index(&out, None), None);
    assert_eq!(key_at(&out, 0), None, "a header carries no key");
    assert_eq!(key_at(&out, 2), None, "nor does a blank");
    assert_eq!(key_at(&out, 99), None, "nor does a row that is not there");
    assert_eq!(filled_index(&out, Some(&key(&m, 0, 0))), Some(1));
    assert_eq!(filled_index(&out, Some(&key(&m, 0, 2))), Some(5));
}

#[test]
fn the_box_is_titled_projects() {
    // Interface spec 5.1: every artboard draws `Projects`, and section 13 carries the open
    // question about `Project Navigator`. The literal, so a change has to be deliberate.
    assert_eq!(PROJECTS_TITLE, "Projects");
}

#[test]
fn a_model_with_no_projects_has_no_rows() {
    let out = rows(
        &Model::new(7),
        &FactRegistry::new(),
        "",
        None,
        Extras::compact(36),
    );
    assert!(out.rows.is_empty(), "the box draws its empty text instead");
    assert_eq!(out.filled, None, "and nothing is filled");
}
