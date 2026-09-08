use domux_core::facts::{Fact, FactKey, FactState, FACT_BRANCH, FACT_PR};
use domux_core::model::Model;
use domux_server::facts::FactRegistry;
use domux_server::render::list_box::{ListBox, ListRow};
use domux_server::render::projects_box::{filled_index, key_at, pr_style, rows, Extras};
use domux_server::render::theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
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

/// The id of the nth workspace of the first project, which is the key of its row.
fn key(m: &Model, project: usize, workspace: usize) -> String {
    m.projects[project].workspaces[workspace].id.to_string()
}

#[test]
fn the_rows_are_the_header_main_a_named_workspace_and_an_untouched_slot() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::compact(36));
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
    assert_eq!(text(&out[1], 0), "main");
    assert_eq!(out[1].lines.len(), 1, "main on its own branch is one line");
    assert_eq!(text(&out[2], 0), "");
    assert!(out[2].is_blank(), "the row between workspaces is a blank");
    assert_eq!(text(&out[3], 0), "auth cleanup");
    assert_eq!(text(&out[3], 1), "feat/auth-cleanup · PR#212");
    assert_eq!(out[3].lines.len(), 2, "the sidebar stops after line 2");
    assert_eq!(text(&out[5], 0), "◌ workspace-2");
    assert_eq!(
        out[5].lines.len(),
        1,
        "an untouched slot drops the branch line that would repeat its handle"
    );
}

#[test]
fn colours_follow_interface_spec_5_2_and_the_pull_request_state() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::compact(36));
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
    assert_eq!(
        out[3].lines[1].spans[0].style.fg,
        Some(theme::PINK),
        "the branch"
    );
    assert_eq!(
        out[3].lines[1].spans[1].style.fg,
        Some(theme::SURFACE1),
        "the middle dot"
    );
    assert_eq!(
        out[3].lines[1].spans[2].style.fg,
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
    let out = rows(&m, &f, "", None, Extras::compact(36));
    assert_eq!(
        text(&out[5], 0),
        "workspace-2",
        "no hollow glyph once the slot has been touched"
    );
    assert_eq!(out[5].lines[0].spans[0].style.fg, Some(theme::TEAL));
    assert_eq!(text(&out[5], 1), "feat/spike");
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
    let out = rows(&m, &f, "", None, Extras::compact(36));
    assert_eq!(text(&out[5], 0), "workspace-2");
    assert_eq!(out[5].lines[0].spans[0].style.fg, Some(theme::TEAL));
    assert_eq!(
        text(&out[5], 1),
        "PR#9",
        "the number alone, with no separator in front of it"
    );
    assert_eq!(out[5].lines[1].spans[0].style.fg, Some(theme::MAUVE));
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
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36));
    assert_eq!(text(&out[3], 0), "auth cleanup");
    assert_eq!(
        out[3].lines.len(),
        1,
        "an absent branch is absent, not the handle and not empty text"
    );
    assert_eq!(text(&out[1], 0), "main");
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
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36));
    assert_eq!(text(&out[3], 0), "workspace-1");
    assert_eq!(out[3].lines[0].spans[0].style.fg, Some(theme::TEAL));
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
    let out = rows(&m, &f, "", None, Extras::compact(36));
    assert_eq!(text(&out[1], 0), "main");
    assert_eq!(
        text(&out[1], 1),
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
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36));
    assert_eq!(text(&out[1], 0), "trunk");
    assert_eq!(
        out[1].lines[0].spans[0].style.fg,
        Some(theme::TEAL),
        "a name is a name, on main as on a slot"
    );
}

#[test]
fn projects_are_listed_alphabetically_whatever_order_they_were_added_in() {
    let mut m = Model::new(7);
    m.add_git_project(PathBuf::from("/repo/zeta"), "main".into())
        .unwrap();
    m.add_git_project(PathBuf::from("/repo/Alpha"), "main".into())
        .unwrap();
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(36));
    assert!(
        text(&out[0], 0).starts_with("ALPHA "),
        "{:?}",
        text(&out[0], 0)
    );
    assert!(out[2].is_blank(), "one blank row before the next header");
    assert!(
        text(&out[3], 0).starts_with("ZETA "),
        "{:?}",
        text(&out[3], 0)
    );
}

#[test]
fn a_project_name_too_wide_for_the_box_is_shortened_and_leaves_no_rule() {
    let mut m = Model::new(7);
    m.add_git_project(
        PathBuf::from("/repo/a-project-with-a-very-long-name-indeed"),
        "main".into(),
    )
    .unwrap();
    let out = rows(&m, &FactRegistry::new(), "", None, Extras::compact(20));
    assert_eq!(text(&out[0], 0), "A-PROJECT-WITH-A-VE…");
    assert_eq!(
        out[0].lines[0].spans[1].content, "",
        "the rule has nothing left to draw"
    );
}

#[test]
fn the_switcher_adds_the_pull_request_title_and_the_tab_list_when_the_width_allows() {
    let (mut m, f) = model_and_facts();
    let w1 = m.projects[0].workspaces[1].id.clone();
    let (t1, _, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
    m.rename_tab(&t1, Some("pr1".into())).unwrap();
    m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
    let out = rows(&m, &f, "", None, Extras::switcher(58));
    assert_eq!(
        text(&out[3], 1),
        "feat/auth-cleanup · PR#212 · Consolidate auth middleware"
    );
    assert_eq!(
        out[3].lines[1].spans[4].style.fg,
        Some(theme::OVERLAY1),
        "the title"
    );
    assert_eq!(text(&out[3], 2), "1 pr1     2");
    assert_eq!(
        out[3].lines[2].spans[0].style.fg,
        Some(theme::OVERLAY0),
        "the tab number"
    );
    assert_eq!(
        out[3].lines[2].spans[2].style.fg,
        Some(theme::OVERLAY1),
        "the tab name"
    );
    let narrow = rows(&m, &f, "", None, Extras::switcher(30));
    assert_eq!(
        text(&narrow[3], 1),
        "feat/auth-cleanup · PR#212",
        "the title goes first when the column is tight"
    );
}

#[test]
fn the_sidebar_shows_no_title_and_no_tab_list_however_much_there_is_to_show() {
    let (mut m, f) = model_and_facts();
    let w1 = m.projects[0].workspaces[1].id.clone();
    let (t1, _, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
    m.rename_tab(&t1, Some("pr1".into())).unwrap();
    let out = rows(&m, &f, "", None, Extras::compact(80));
    assert_eq!(
        text(&out[3], 1),
        "feat/auth-cleanup · PR#212",
        "no title in the sidebar, however wide the box happens to be"
    );
    assert_eq!(out[3].lines.len(), 2, "and no tab list");
}

#[test]
fn a_workspace_with_no_tabs_has_no_tab_list_line() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::switcher(58));
    assert_eq!(
        out[3].lines.len(),
        2,
        "a workspace nobody has opened a tab in draws no empty tab line"
    );
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
    let out = rows(&m, &f, "", None, Extras::switcher(26));
    let line2 = text(&out[3], 1);
    // 26 cells: the branch keeps what the number (6) and the separator (3) leave it.
    assert_eq!(line2, "feat/a-very-long… · PR#212");
    assert!(
        line2.ends_with("PR#212"),
        "the number is never cut (interface spec 12.16)"
    );
    assert_eq!(text(&out[3], 0), "a very long workspace nam…");
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
    let out = rows(&m, &f, "", None, Extras::compact(9));
    assert_eq!(
        text(&out[3], 1),
        "PR#212",
        "the number alone, with no leading separator where the branch used to be"
    );
    assert_eq!(out[3].lines[1].spans.len(), 1);
}

#[test]
fn the_filter_keeps_matching_workspaces_with_their_header_and_says_so_when_nothing_matches() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "auth", None, Extras::compact(36));
    assert_eq!(out.len(), 2, "the header and the one match");
    assert_eq!(text(&out[1], 0), "auth cleanup");
    let branch = rows(&m, &f, "FEAT/", None, Extras::compact(36));
    assert_eq!(
        branch.len(),
        2,
        "the filter matches the branch too, without case"
    );
    assert!(
        rows(&m, &f, "zzz", None, Extras::compact(36)).is_empty(),
        "no matches means no rows; the box draws its empty text"
    );
}

#[test]
fn the_filter_matches_the_handle_the_number_and_the_project_name() {
    let (m, f) = model_and_facts();
    let handle = rows(&m, &f, "workspace-2", None, Extras::compact(36));
    assert_eq!(handle.len(), 2, "the header and the slot");
    assert_eq!(text(&handle[1], 0), "◌ workspace-2");
    let number = rows(&m, &f, "pr#212", None, Extras::compact(36));
    assert_eq!(number.len(), 2, "the pull request number is matched too");
    assert_eq!(text(&number[1], 0), "auth cleanup");
    let project = rows(&m, &f, "audrey", None, Extras::compact(36));
    assert_eq!(
        project.len(),
        4,
        "a project name keeps every workspace under it, with no blank between them"
    );
    assert_eq!(
        vec![
            text(&project[1], 0),
            text(&project[2], 0),
            text(&project[3], 0)
        ],
        vec!["main", "auth cleanup", "◌ workspace-2"],
    );
}

#[test]
fn the_filled_row_takes_the_bold_and_the_bright_text_the_box_cannot_give_it() {
    // Interface spec 5.3 is drawn by two files: `ListBox` lays the band and removes the
    // dimming, and the row builder decides the bold and the `text` colour. This pins the
    // half that lives here, and that the band is not built here.
    let (m, f) = model_and_facts();
    let named = rows(&m, &f, "", Some(&key(&m, 0, 1)), Extras::compact(36));
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

    let on_main = rows(&m, &f, "", Some(&key(&m, 0, 0)), Extras::compact(36));
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

    let on_slot = rows(&m, &f, "", Some(&key(&m, 0, 2)), Extras::compact(36));
    let style = on_slot[5].lines[0].spans[0].style;
    assert_eq!(
        style.fg,
        Some(theme::TEXT),
        "a slot handle goes from dim to text"
    );
    assert!(!style.add_modifier.contains(Modifier::BOLD));

    let none = rows(&m, &f, "", None, Extras::compact(36));
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
    let out = rows(&m, &f, "", Some("w_ffff"), Extras::compact(36));
    assert_eq!(out[3].lines[0].spans[0].style.fg, Some(theme::TEAL));
    assert!(!out[3].lines[0].spans[0]
        .style
        .add_modifier
        .contains(Modifier::BOLD));
    assert_eq!(filled_index(&out, Some("w_ffff")), None);
}

#[test]
fn filled_index_and_key_at_name_the_same_row() {
    let (m, f) = model_and_facts();
    let out = rows(&m, &f, "", None, Extras::compact(36));
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
fn a_model_with_no_projects_has_no_rows() {
    let out = rows(
        &Model::new(7),
        &FactRegistry::new(),
        "",
        None,
        Extras::compact(36),
    );
    assert!(out.is_empty(), "the box draws its empty text instead");
}

#[test]
fn a_tab_list_wider_than_the_box_is_cut_by_the_box_with_its_colours_intact() {
    // The tab list is built whole, because it has no order of importance to express: the
    // box cuts it, span by span, the way it cuts every other line.
    let (mut m, f) = model_and_facts();
    let w1 = m.projects[0].workspaces[1].id.clone();
    for name in ["review", "tests", "notes"] {
        let (t, _, _) = m.create_tab(&w1, PathBuf::from("/w1")).unwrap();
        m.rename_tab(&t, Some(name.into())).unwrap();
    }
    let out = rows(&m, &f, "", None, Extras::switcher(20));
    assert_eq!(
        text(&out[3], 2),
        "1 review     2 tests     3 notes",
        "the row keeps every tab"
    );
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 8));
    ListBox {
        title: "Projects",
        rows: &out,
        filled: None,
        focused: false,
        scroll: 0,
        empty_text: "",
    }
    .render(Rect::new(0, 0, 20, 8), &mut buf);
    let drawn: String = (0..20).map(|x| buf[(x, 6)].symbol().to_string()).collect();
    assert_eq!(drawn, "│1 review     2 te…│", "{drawn}");
    assert_eq!(
        buf[(1, 6)].fg,
        theme::OVERLAY0,
        "the number keeps its colour through the cut"
    );
    assert_eq!(buf[(3, 6)].fg, theme::OVERLAY1, "and so does the name");
}
