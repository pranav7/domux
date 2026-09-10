use domux_server::render::list_box::{
    filter_rows, footer_area, scroll_to_show, text_area, ListBox, ListRow, OVERLAY_PAD, SIDEBAR_PAD,
};
use domux_server::render::theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

/// The first span of a row's first line, which is enough to tell the fixture's rows apart.
fn text(r: &ListRow) -> String {
    r.lines[0]
        .spans
        .first()
        .map(|s| s.content.to_string())
        .unwrap_or_default()
}

fn texts(rs: &[ListRow]) -> Vec<String> {
    rs.iter().map(text).collect()
}

fn rows() -> Vec<ListRow> {
    vec![
        ListRow::header(vec![Line::from("PROJ ─────────")]),
        ListRow::selectable("w_0001", "main", vec![Line::from("main")]),
        ListRow::blank(),
        ListRow::selectable(
            "w_c3a1",
            "auth cleanup feat/auth-cleanup",
            vec![Line::from("auth cleanup"), Line::from("feat/auth-cleanup")],
        ),
        ListRow::blank(),
        ListRow::selectable("w_0002", "workspace-2", vec![Line::from("◌ workspace-2")]),
    ]
}

/// MUX-12: the overlay's padding is two cells in from each border and a blank row at each
/// end, where the sidebar's is one cell and no blank rows. MUX-16: it also keeps the box's
/// last row for the footer, which the caller draws and `ListBox` leaves alone.
///
/// The fill band is asserted beside the text, because the pad is inside the band and not
/// beside it: a filled row still reaches both borders (decision record 0012), and only its
/// text starts two cells in.
#[test]
fn the_overlay_pad_keeps_a_blank_row_a_footer_row_and_two_cells_at_each_side() {
    let area = Rect::new(0, 0, 22, 10);
    let mut buf = Buffer::empty(area);
    let scroll = ListBox {
        title: "Projects",
        rows: &rows(),
        filled: Some(1),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: OVERLAY_PAD,
    }
    .render(area, &mut buf);
    assert_eq!(scroll, 0);
    assert_eq!(row(&buf, 0), "┌ Projects ──────────┐");
    assert_eq!(
        row(&buf, 1),
        "│                    │",
        "the blank row under the rule"
    );
    assert_eq!(row(&buf, 2), "│  PROJ ─────────    │");
    assert_eq!(row(&buf, 3), "│  main              │");
    assert_eq!(
        row(&buf, 7),
        "│                    │",
        "and one under the rows"
    );
    assert_eq!(
        row(&buf, 8),
        "│                    │",
        "then the footer's row, which this box draws nothing in"
    );
    assert_eq!(row(&buf, 9), "└────────────────────┘");
    assert_eq!(
        text_area(area, OVERLAY_PAD),
        Rect::new(1, 2, 20, 5),
        "the rectangle the pointer is measured against is the one that was drawn"
    );
    assert_eq!(
        footer_area(area, OVERLAY_PAD),
        Some(Rect::new(3, 8, 16, 1)),
        "the footer's row is the last inside the border, in by the side pad at each end so it \
         starts in the column the rows above it start in"
    );
    assert_eq!(
        footer_area(area, SIDEBAR_PAD),
        None,
        "and the sidebar's boxes keep no such row: its hint row is under both of them"
    );
    assert_eq!(
        (buf[(1, 3)].bg, buf[(20, 3)].bg),
        (theme::SURFACE0, theme::SURFACE0),
        "the fill still reaches both borders"
    );
}

#[test]
fn the_box_draws_its_title_its_rows_and_one_filled_row() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 22, 9));
    let scroll = ListBox {
        title: "Projects",
        rows: &rows(),
        filled: Some(3),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 22, 9), &mut buf);
    assert_eq!(scroll, 0);
    assert_eq!(row(&buf, 0), "┌ Projects ──────────┐");
    assert_eq!(row(&buf, 1), "│ PROJ ─────────     │");
    assert_eq!(row(&buf, 2), "│ main               │");
    assert_eq!(row(&buf, 3), "│                    │");
    assert_eq!(row(&buf, 4), "│ auth cleanup       │");
    assert_eq!(row(&buf, 5), "│ feat/auth-cleanup  │");
    assert_eq!(row(&buf, 8), "└────────────────────┘");
    for x in 1..21 {
        assert_eq!(
            buf[(x, 4)].bg,
            theme::SURFACE0,
            "the fill covers the whole first line of the row"
        );
    }
    // The literal, not the token: surface0 is what interface spec 9.1 names for the fill, so
    // a test that only compared the two would pass for any value the token took.
    assert_eq!(buf[(1, 4)].bg, Color::Rgb(0x31, 0x32, 0x44));
    assert_eq!(
        buf[(1, 5)].bg,
        Color::Reset,
        "line 2 is not filled (interface spec 5.3)"
    );
    assert_eq!(
        buf[(0, 3)].bg,
        Color::Reset,
        "the fill stops at the row it is on"
    );
    assert_eq!(
        buf[(0, 0)].fg,
        theme::ACCENT,
        "a focused box takes the accent border"
    );
}

#[test]
fn the_box_draws_at_the_area_it_is_given_and_touches_nothing_outside_it() {
    // Every other render test here passes an area at the origin into a buffer of exactly
    // that size, so a `render` that ignored `area.x` and `area.y` would pass all of them.
    // The sidebar draws at x = 0; the switcher overlay draws the same box centred.
    let mut buf = Buffer::empty(Rect::new(0, 0, 26, 12));
    ListBox {
        title: "Projects",
        rows: &rows(),
        filled: Some(3),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(3, 2, 20, 9), &mut buf);
    assert_eq!(
        row(&buf, 1),
        " ".repeat(26),
        "the row above the box is untouched"
    );
    assert_eq!(row(&buf, 2), "   ┌ Projects ────────┐   ");
    assert_eq!(row(&buf, 3), "   │ PROJ ─────────   │   ");
    assert_eq!(row(&buf, 6), "   │ auth cleanup     │   ");
    assert_eq!(row(&buf, 10), "   └──────────────────┘   ");
    assert_eq!(row(&buf, 11), " ".repeat(26), "and the row below it");
    assert_eq!(
        buf[(4, 6)].bg,
        theme::SURFACE0,
        "the fill lands at the offset too"
    );
    assert_eq!(
        buf[(2, 6)].bg,
        Color::Reset,
        "and stops at the box's own left edge"
    );
}

#[test]
fn a_box_that_is_not_focused_keeps_the_plain_border_and_still_fills_the_current_row() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 9));
    ListBox {
        title: "Projects",
        rows: &rows(),
        filled: Some(1),
        focused: false,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 20, 9), &mut buf);
    assert_eq!(buf[(0, 0)].fg, theme::SURFACE2);
    assert!(
        !buf[(2, 0)].modifier.contains(Modifier::BOLD),
        "an unfocused title is plain"
    );
    assert_eq!(
        buf[(1, 2)].bg,
        theme::SURFACE0,
        "the current row is filled while focus is elsewhere"
    );
}

#[test]
fn scrolling_keeps_the_whole_filled_row_in_view_and_moves_as_little_as_it_can() {
    let rows = rows();
    // Total 7 lines: the rows occupy lines 0, 1, 2, 3-4, 5, 6. With `height` visible lines
    // and a scroll of `s`, the view is lines `s` to `s + height - 1`.
    assert_eq!(
        scroll_to_show(&rows, Some(1), 5, 0),
        0,
        "already in view: do not move"
    );
    assert_eq!(
        scroll_to_show(&rows, Some(3), 5, 0),
        0,
        "the two-line row ends on line 4, the last line of a five-line view"
    );
    assert_eq!(
        scroll_to_show(&rows, Some(3), 4, 0),
        1,
        "one line shorter, the two-line row's last line is one past the view"
    );
    assert_eq!(
        scroll_to_show(&rows, Some(5), 3, 0),
        4,
        "scroll down to the last row"
    );
    assert_eq!(
        scroll_to_show(&rows, Some(1), 3, 4),
        1,
        "scroll back up to the row's first line"
    );
    assert_eq!(
        scroll_to_show(&rows, Some(0), 3, 4),
        0,
        "the cursor on the first row scrolls back to the top"
    );
    assert_eq!(
        scroll_to_show(&rows, None, 3, 4),
        4,
        "no filled row leaves the scroll alone"
    );
    assert_eq!(
        scroll_to_show(&rows, Some(5), 20, 0),
        0,
        "a box taller than its rows never scrolls"
    );
    assert_eq!(
        scroll_to_show(&rows, None, 3, 9),
        4,
        "a remembered scroll past the last line is pulled back to it"
    );
}

#[test]
fn scroll_to_show_answers_for_an_empty_list_and_a_row_index_that_is_gone() {
    // The cursor's row can be deleted between the client's last frame and this one, so the
    // index arrives out of range rather than never arriving.
    assert_eq!(scroll_to_show(&[], Some(3), 5, 2), 0);
    assert_eq!(scroll_to_show(&[], None, 5, 2), 0);
    assert_eq!(scroll_to_show(&rows(), Some(99), 3, 0), 4);
}

#[test]
fn a_row_taller_than_the_box_shows_its_first_line_so_the_fill_stays_visible() {
    let rows = vec![
        ListRow::selectable("w_1", "one", vec![Line::from("one")]),
        ListRow::selectable(
            "w_2",
            "tall",
            vec![Line::from("a"), Line::from("b"), Line::from("c")],
        ),
    ];
    // Lines 0, then 1-3. The three-line row cannot fit in a two-line view.
    assert_eq!(
        scroll_to_show(&rows, Some(1), 2, 0),
        1,
        "show the row's first line, the line the fill is on"
    );

    let mut buf = Buffer::empty(Rect::new(0, 0, 8, 4));
    let scroll = ListBox {
        title: "T",
        rows: &rows,
        filled: Some(1),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 8, 4), &mut buf);
    assert_eq!(scroll, 1);
    assert_eq!(row(&buf, 1), "│ a    │");
    assert_eq!(row(&buf, 2), "│ b    │");
    assert_eq!(buf[(1, 1)].bg, theme::SURFACE0, "the fill is in view");
}

#[test]
fn a_scrolled_box_starts_at_the_scroll_line_and_never_draws_a_sticky_header() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 22, 5));
    let scroll = ListBox {
        title: "Projects",
        rows: &rows(),
        filled: Some(5),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 22, 5), &mut buf);
    assert_eq!(scroll, 4);
    assert_eq!(
        row(&buf, 1),
        "│ feat/auth-cleanup  │",
        "the view starts at the scroll line"
    );
    assert_eq!(row(&buf, 2), "│                    │");
    assert_eq!(row(&buf, 3), "│ ◌ workspace-2      │");
    assert!(
        !row(&buf, 1).contains("PROJ"),
        "the header scrolls away with its rows"
    );
    assert_eq!(
        row(&buf, 4),
        "└────────────────────┘",
        "nothing runs past the box"
    );
}

#[test]
fn a_row_above_the_scroll_line_is_not_drawn() {
    // The top edge and the bottom edge are two clips, so a renderer can pass one and fail the
    // other: the test above draws the tail of the list, this one draws its head.
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
    let scroll = ListBox {
        title: "Projects",
        rows: &rows(),
        filled: Some(1),
        focused: true,
        scroll: 3,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 20, 4), &mut buf);
    assert_eq!(
        scroll, 1,
        "the cursor above the view pulls it back to the row"
    );
    assert_eq!(row(&buf, 1), "│ main             │");
    assert_eq!(
        row(&buf, 2),
        "│                  │",
        "the blank row after it"
    );
    assert_eq!(
        row(&buf, 3),
        "└──────────────────┘",
        "the header is above the view and the two-line row is below it"
    );
}

#[test]
fn an_empty_box_says_what_is_missing_rather_than_drawing_nothing() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 34, 5));
    let scroll = ListBox {
        title: "Projects",
        rows: &[],
        filled: None,
        focused: true,
        scroll: 2,
        empty_text: "No projects yet. domux2 open .",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 34, 5), &mut buf);
    assert_eq!(scroll, 0, "there is nothing to scroll past");
    assert_eq!(row(&buf, 1), "│ No projects yet. domux2 open . │");
    assert_eq!(buf[(2, 1)].fg, theme::OVERLAY0);
    assert_eq!(buf[(2, 1)].fg, Color::Rgb(0x6c, 0x70, 0x86));
}

/// An empty text longer than the box is cut to it, with the mark that says it was cut.
///
/// M3 made the empty text wrap over the rows a box has, and this box has one row inside its
/// border, so there is nowhere to wrap to. The last row it can draw is filled from everything
/// that is left rather than from the next wrapped word, so a box too short to wrap shows
/// exactly what it showed before M3 rather than one word of it.
#[test]
fn an_empty_text_wider_than_the_box_is_cut_rather_than_written_over_the_border() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    ListBox {
        title: "Projects",
        rows: &[],
        filled: None,
        focused: true,
        scroll: 0,
        empty_text: "No projects yet. domux2 open .",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 12, 3), &mut buf);
    assert_eq!(row(&buf, 1), "│ No proj… │");
}

/// The same text in a box with rows to spare: every word of it, over as many lines as it
/// takes, with no mark because nothing was cut.
#[test]
fn an_empty_text_wider_than_the_box_wraps_onto_the_rows_below_it() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 14, 5));
    ListBox {
        title: "Projects",
        rows: &[],
        filled: None,
        focused: true,
        scroll: 0,
        empty_text: "No projects yet. open",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 14, 5), &mut buf);
    // 14 wide and not 12: the pad takes a column off each side, and at 12 the last line has
    // one cell too few and ends in the mark, which is the other test's case.
    assert_eq!(row(&buf, 1), "│ No         │");
    assert_eq!(row(&buf, 2), "│ projects   │");
    assert_eq!(row(&buf, 3), "│ yet. open  │");
}

#[test]
fn a_line_wider_than_the_box_is_cut_by_grapheme_with_an_ellipsis() {
    let rows = vec![ListRow::selectable(
        "w_1",
        "x",
        vec![Line::from("漢字漢字漢字漢字漢字")],
    )];
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    ListBox {
        title: "Projects",
        rows: &rows,
        filled: Some(0),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 12, 3), &mut buf);
    // The cell after a wide grapheme is reset to a space, as `render_grid` and `put` leave
    // it, so a row read cell by cell has a space between each pair.
    assert_eq!(
        row(&buf, 1),
        "│ 漢 字 漢 …  │",
        "no half of a wide character survives (principle 6)"
    );
    assert_eq!(buf[(8, 1)].symbol(), "…");
    assert_eq!(
        buf[(9, 1)].symbol(),
        " ",
        "the fourth wide grapheme does not fit and is dropped whole"
    );
    assert_eq!(buf[(11, 1)].symbol(), "│", "the border survives");
    assert_eq!(
        buf[(3, 1)].bg,
        theme::SURFACE0,
        "the spacer cell after a wide grapheme keeps the fill, so the band has no hole"
    );
}

#[test]
fn nothing_is_drawn_after_the_span_that_was_cut() {
    // "漢字漢字" is 8 cells in the 6 of room a ten-cell box leaves after its border and its
    // pads, so the cut keeps two of them and the ellipsis, 5 cells, and leaves one spare. A
    // later span drawn into that spare cell reads as text that survived the cut when it fits
    // there, and as a second ellipsis when it does not.
    let draw = |tail: &'static str| {
        let rows = vec![ListRow::selectable(
            "w_1",
            "x",
            vec![Line::from(vec![Span::raw("漢字漢字"), Span::raw(tail)])],
        )];
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        ListBox {
            title: "T",
            rows: &rows,
            filled: None,
            focused: true,
            scroll: 0,
            empty_text: "",
            pad: SIDEBAR_PAD,
        }
        .render(Rect::new(0, 0, 10, 3), &mut buf);
        (row(&buf, 1), buf[(7, 1)].symbol().to_string())
    };

    let (line, spare) = draw("TAIL");
    assert_eq!(
        line, "│ 漢 字 …  │",
        "a tail too wide for the spare cell would read ……"
    );
    assert_eq!(spare, " ", "the spare cell stays empty");

    let (line, spare) = draw("T");
    assert_eq!(
        line, "│ 漢 字 …  │",
        "a tail that fits the spare cell would read …T"
    );
    assert_eq!(spare, " ", "the spare cell stays empty");
}

#[test]
fn spans_that_fit_are_drawn_one_after_another_in_their_own_styles() {
    let rows = vec![ListRow::selectable(
        "w_1",
        "x",
        vec![Line::from(vec![
            Span::styled("auth", Style::default().fg(theme::TEAL)),
            Span::raw(" "),
            Span::styled("feat/auth", Style::default().fg(theme::PINK)),
        ])],
    )];
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
    ListBox {
        title: "T",
        rows: &rows,
        filled: None,
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 20, 3), &mut buf);
    assert_eq!(row(&buf, 1), "│ auth feat/auth   │");
    assert_eq!(buf[(2, 1)].fg, theme::TEAL, "the workspace name is teal");
    assert_eq!(buf[(7, 1)].fg, theme::PINK, "the branch name is pink");
}

#[test]
fn the_fill_brightens_the_row_and_leaves_the_rows_around_it_alone() {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let line = || {
        Line::from(vec![
            Span::styled("main", bold),
            Span::raw(" "),
            Span::styled("dim", dim),
        ])
    };
    let rows = vec![
        ListRow::selectable("w_1", "one", vec![line()]),
        ListRow::selectable("w_2", "two", vec![line()]),
    ];
    let mut buf = Buffer::empty(Rect::new(0, 0, 14, 4));
    ListBox {
        title: "T",
        rows: &rows,
        filled: Some(0),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 14, 4), &mut buf);
    assert!(
        buf[(2, 1)].modifier.contains(Modifier::BOLD),
        "bold text on the filled row stays bold"
    );
    assert!(
        !buf[(7, 1)].modifier.contains(Modifier::DIM),
        "dim text on the filled row loses its dimming (interface spec 5.3)"
    );
    assert_eq!(
        buf[(7, 1)].bg,
        theme::SURFACE0,
        "the text takes the fill too"
    );
    assert!(
        buf[(7, 2)].modifier.contains(Modifier::DIM),
        "a row without the fill keeps its dimming"
    );
    assert_eq!(buf[(7, 2)].bg, Color::Reset);
}

#[test]
fn a_row_cannot_write_over_the_border_with_text_a_terminal_cannot_draw() {
    // A control character measures one cell in `display_width` and draws none, so a budget
    // measured before it is dropped cuts one cell early. The eight cells of room a twelve-cell
    // box leaves after its border and its pads hold "abcdefg…", not "abcdef…". The write is
    // clipped at the border as a second layer.
    let rows = vec![ListRow::selectable(
        "w_1",
        "x",
        vec![Line::from(vec![Span::raw("a\u{7}b\u{200b}cdefghijklmnop")])],
    )];
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    ListBox {
        title: "T",
        rows: &rows,
        filled: Some(0),
        focused: true,
        scroll: 0,
        empty_text: "",
        pad: SIDEBAR_PAD,
    }
    .render(Rect::new(0, 0, 12, 3), &mut buf);
    assert_eq!(
        row(&buf, 1),
        "│ abcdefg… │",
        "the cut is measured on the cells that will be drawn"
    );
    assert_eq!(buf[(11, 1)].symbol(), "│", "the right border survives");
    assert_eq!(buf[(0, 1)].symbol(), "│", "the left border survives");
    for x in 0..12u16 {
        assert!(
            !buf[(x, 1)].symbol().chars().any(char::is_control),
            "cell x{x} holds a control character"
        );
    }
    assert_eq!(row(&buf, 2), "└──────────┘");
}

#[test]
fn a_box_with_no_room_inside_draws_nothing_and_keeps_the_scroll_it_was_given() {
    for (w, h) in [(0u16, 0u16), (2, 5), (20, 2), (1, 1)] {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 5));
        let scroll = ListBox {
            title: "Projects",
            rows: &rows(),
            filled: Some(5),
            focused: true,
            scroll: 3,
            empty_text: "nothing",
            pad: SIDEBAR_PAD,
        }
        .render(Rect::new(0, 0, w, h), &mut buf);
        assert_eq!(scroll, 3, "a {w}x{h} box has no view to correct");
    }
}

#[test]
fn the_row_constructors_say_which_rows_take_the_cursor() {
    let header = ListRow::header(vec![Line::from("PROJ")]);
    assert_eq!(header.key, None, "the cursor never rests on a header");
    assert_eq!(header.height(), 1);
    assert!(!header.is_blank(), "a header has something to read");

    let blank = ListRow::blank();
    assert_eq!(blank.key, None);
    assert_eq!(blank.height(), 1, "a blank row still takes a line");
    assert!(blank.is_blank());

    let row = ListRow::selectable(
        "w_c3a1",
        "Auth Cleanup",
        vec![Line::from("a"), Line::from("b")],
    );
    assert_eq!(row.key.as_deref(), Some("w_c3a1"));
    assert_eq!(
        row.filter_text, "auth cleanup",
        "the filter text is lower-cased once, at the constructor"
    );
    assert_eq!(row.height(), 2);
    assert!(!row.is_blank());

    assert!(
        ListRow::header(vec![Line::from("   ")]).is_blank(),
        "a row of spaces has nothing to read, so it is a separator and not a header"
    );
    assert!(
        !ListRow::selectable("w_1", "", vec![Line::from("   ")]).is_blank(),
        "a row the cursor rests on is never a separator, whatever it draws"
    );

    // A row is measured in `u16` because that is what a screen is measured in. `as u16` would
    // wrap 65_537 lines back to 1 and scroll to the wrong line, so the conversion saturates.
    let huge = ListRow::header(vec![Line::from(""); 65_537]);
    assert_eq!(
        huge.height(),
        u16::MAX,
        "a row taller than u16 saturates rather than wrapping to 1"
    );
}

#[test]
fn the_filter_keeps_matching_rows_under_their_own_header() {
    let groups = vec![
        ListRow::header(vec![Line::from("AUDREY")]),
        ListRow::selectable("w_1", "main", vec![Line::from("main")]),
        ListRow::selectable("w_2", "auth cleanup", vec![Line::from("auth cleanup")]),
        ListRow::blank(),
        ListRow::header(vec![Line::from("DOMUX")]),
        ListRow::selectable("w_3", "main", vec![Line::from("main")]),
        ListRow::blank(),
        ListRow::header(vec![Line::from("NOTES")]),
        ListRow::selectable("w_4", "drafts", vec![Line::from("drafts")]),
    ];

    let kept = filter_rows(&groups, "MAIN");
    assert_eq!(
        texts(&kept),
        vec!["AUDREY", "main", "", "DOMUX", "main"],
        "matching without case, each under its header, with one blank between the groups"
    );
    assert_eq!(kept[0].key, None, "the header is still a header");
    assert!(
        kept[2].is_blank(),
        "the separator is a blank row, not a header"
    );

    assert_eq!(
        texts(&filter_rows(&groups, "auth")),
        vec!["AUDREY", "auth cleanup"],
        "a header with no surviving rows goes with them"
    );
    assert!(
        filter_rows(&groups, "zzz").is_empty(),
        "no match leaves no header behind"
    );
    assert_eq!(
        texts(&filter_rows(&groups, " main ")),
        vec!["AUDREY", "main", "", "DOMUX", "main"],
        "the filter is trimmed before it is matched"
    );
}

#[test]
fn matches_are_spaced_the_way_the_builder_spaces_them_rather_than_evenly() {
    // The builder puts a blank before a header and, inside a group, only after a row that
    // said more than its name. The filter rebuilds to the same rule, so `/` changes what the
    // list holds and never its shape: a row of three lines still ends somewhere the reader
    // can see, and two rows of one line still read as one block.
    let groups = vec![
        ListRow::header(vec![Line::from("AUDREY")]),
        ListRow::selectable(
            "w_1",
            "auth cleanup",
            vec![Line::from("auth cleanup"), Line::from("feat/auth-cleanup")],
        ),
        ListRow::blank(),
        ListRow::selectable("w_2", "auth notes", vec![Line::from("auth notes")]),
        ListRow::selectable("w_3", "auth drafts", vec![Line::from("auth drafts")]),
        ListRow::selectable("w_4", "release", vec![Line::from("release")]),
        ListRow::blank(),
        ListRow::header(vec![Line::from("DOMUX")]),
        ListRow::selectable("w_5", "auth docs", vec![Line::from("auth docs")]),
    ];

    let kept = filter_rows(&groups, "auth");
    assert_eq!(
        texts(&kept),
        vec![
            "AUDREY",
            "auth cleanup",
            "",
            "auth notes",
            "auth drafts",
            "",
            "DOMUX",
            "auth docs"
        ],
        "a blank after the two-line match, none between the two one-line ones, and one \
         before the next header"
    );
    assert!(
        kept[2].is_blank(),
        "the row after a match of more than one line is a blank"
    );
    assert!(
        kept[5].is_blank(),
        "and so is the row before the next header"
    );
    assert_eq!(kept[6].key, None, "which is followed by the header itself");
    assert!(
        !kept[0].is_blank(),
        "and the list never opens on a blank row"
    );
}

#[test]
fn no_filter_returns_the_list_untouched_rather_than_a_list_that_matched_everything() {
    // On a list whose rows are separated by blanks, "keep everything" and "match every row"
    // are different answers: matching drops the blanks and rebuilds its own between groups.
    // So compare the rows, not how many came back.
    let rows = rows();
    assert_eq!(
        texts(&filter_rows(&rows, "")),
        texts(&rows),
        "an empty filter returns the list untouched, blank rows and all"
    );
    assert_eq!(
        texts(&filter_rows(&rows, "   ")),
        texts(&rows),
        "a filter of only spaces is no filter"
    );
}

#[test]
fn a_blank_row_before_a_match_is_dropped_rather_than_kept_as_its_header() {
    // The unfiltered Projects box puts a blank after a workspace that drew more than one
    // line, so the row just above a match can be a blank rather than the project's header.
    let rows = rows();
    assert_eq!(
        texts(&filter_rows(&rows, "workspace-2")),
        vec!["PROJ ─────────", "◌ workspace-2"],
        "the project header leads the match, and the blank above it is gone"
    );
    assert_eq!(
        texts(&filter_rows(&rows, "feat/auth")),
        vec!["PROJ ─────────", "auth cleanup"],
        "the filter matches the branch in the row's filter text, not only its first line"
    );
}

#[test]
fn the_branch_and_workspace_colours_are_the_hexes_the_spec_names() {
    // The literals, not the tokens: interface spec 9.1 fixes these two hexes, and comparing
    // a token against itself would pass for any value.
    assert_eq!(theme::PINK, Color::Rgb(0xe3, 0xb4, 0xd8));
    assert_eq!(theme::TEAL, Color::Rgb(0x93, 0xe2, 0xd5));
    // 5.3 names this one: the filled row's dim text goes to `text`.
    assert_eq!(theme::TEXT, Color::Rgb(0xcd, 0xd6, 0xf4));
}
