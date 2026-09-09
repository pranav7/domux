use domux_core::model::Rect as LRect;
use domux_server::render::boxed::{put, put_within, Boxed};
use domux_server::render::pane_box::{cursor_position, render_grid, Selection};
use domux_server::render::{theme, to_rect};
use domux_term::{Attrs, Color, Cursor, Grid, Rgb, Size};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn boxed_draws_title_one_cell_in_and_the_flag_at_the_right_end() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
    let inner = Boxed {
        title: "zsh",
        flag: Some("zoomed"),
        focused: false,
    }
    .render(Rect::new(0, 0, 20, 3), &mut buf);
    assert_eq!(row(&buf, 0), "┌ zsh ───── zoomed ┐");
    assert_eq!(row(&buf, 1), "│                  │");
    assert_eq!(row(&buf, 2), "└──────────────────┘");
    assert_eq!(inner, Rect::new(1, 1, 18, 1));
    assert_eq!(buf[(0, 0)].fg, theme::SURFACE2);
    assert_eq!(buf[(2, 0)].fg, theme::OVERLAY1);
    assert!(!buf[(2, 0)].modifier.contains(Modifier::BOLD));
}

#[test]
fn focused_box_uses_the_accent_and_a_bold_title() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    Boxed {
        title: "sh",
        flag: None,
        focused: true,
    }
    .render(Rect::new(0, 0, 12, 3), &mut buf);
    assert_eq!(row(&buf, 0), "┌ sh ──────┐");
    assert_eq!(buf[(0, 0)].fg, theme::ACCENT);
    assert_eq!(buf[(2, 0)].fg, theme::ACCENT);
    assert!(buf[(2, 0)].modifier.contains(Modifier::BOLD));
    assert!(
        !buf[(6, 0)].modifier.contains(Modifier::BOLD),
        "the rule is plain weight"
    );
}

#[test]
fn long_titles_are_truncated_by_grapheme_and_tiny_boxes_do_not_panic() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
    Boxed {
        title: "漢字漢字漢字",
        flag: None,
        focused: false,
    }
    .render(Rect::new(0, 0, 10, 3), &mut buf);
    // `row` reads one symbol per cell, and `put` calls `reset()` on the cell after each
    // wide grapheme, which ratatui renders as a space. So a 10-cell row is 10 symbols:
    // the box corner, the title's leading space, 漢 and its spacer, 字 and its spacer, the
    // ellipsis, the title's trailing space, one rule cell, and the closing corner.
    assert_eq!(row(&buf, 0), "┌ 漢 字 … ─┐");
    let mut tiny = Buffer::empty(Rect::new(0, 0, 2, 1));
    let inner = Boxed {
        title: "x",
        flag: Some("zoomed"),
        focused: true,
    }
    .render(Rect::new(0, 0, 2, 1), &mut tiny);
    assert_eq!(inner.width, 0);
}

#[test]
fn grid_cells_map_to_symbols_styles_wide_spacers_and_selection() {
    let mut g = Grid::new(Size { cols: 6, rows: 2 });
    let a = g.cell_mut(0, 0);
    a.text = "a".into();
    a.attrs = Attrs::BOLD;
    a.fg = Color::Indexed(1);
    let wide = g.cell_mut(0, 1);
    wide.text = "漢".into();
    wide.width = 2;
    g.cell_mut(0, 2).width = 0;
    let x = g.cell_mut(0, 3);
    x.text = "x".into();
    x.bg = Color::Rgb(Rgb { r: 1, g: 2, b: 3 });
    let mut buf = Buffer::empty(Rect::new(0, 0, 8, 4));
    let inner = Rect::new(1, 1, 6, 2);
    render_grid(
        &g,
        Some(&Selection {
            start: (0, 3),
            end: (1, 1),
        }),
        inner,
        &mut buf,
    );
    assert_eq!(buf[(1, 1)].symbol(), "a");
    assert!(buf[(1, 1)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(1, 1)].fg, ratatui::style::Color::Indexed(1));
    assert_eq!(buf[(2, 1)].symbol(), "漢");
    assert_eq!(
        buf[(3, 1)].symbol(),
        " ",
        "spacer cell is reset so the diff skips it"
    );
    assert_eq!(buf[(4, 1)].bg, ratatui::style::Color::Rgb(1, 2, 3));
    assert!(
        buf[(4, 1)].modifier.contains(Modifier::REVERSED),
        "selected"
    );
    assert!(
        buf[(2, 2)].modifier.contains(Modifier::REVERSED),
        "selection continues on the next row"
    );
    assert!(
        !buf[(3, 2)].modifier.contains(Modifier::REVERSED),
        "selection ends at its end column"
    );
    assert!(!buf[(1, 1)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn cursor_position_offsets_by_the_inner_area_and_hides_when_invisible() {
    let inner = Rect::new(11, 6, 18, 8);
    let c = Cursor {
        row: 2,
        col: 3,
        ..Cursor::default()
    };
    assert_eq!(cursor_position(inner, &c), Some(Position { x: 14, y: 8 }));
    assert_eq!(
        cursor_position(
            inner,
            &Cursor {
                visible: false,
                ..c
            }
        ),
        None
    );
    assert_eq!(cursor_position(inner, &Cursor { col: 40, ..c }), None);
    assert_eq!(
        to_rect(LRect {
            x: 1,
            y: 2,
            width: 3,
            height: 4
        }),
        Rect::new(1, 2, 3, 4)
    );
}

/// A pane's title is whatever the program in it emitted with OSC 0 or OSC 2, so these two
/// inputs are program-controlled and reach `Boxed` unfiltered once a caller exists.
#[test]
fn a_zero_width_title_stays_inside_its_box() {
    // 40 zero-width spaces measure 0 cells, so they satisfy any budget, and `put` then gives
    // each one a cell. Before the fix this wrote 42 cells from x1 in a 5-wide box.
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 3));
    let title = "\u{200b}".repeat(40);
    Boxed {
        title: &title,
        flag: None,
        focused: false,
    }
    .render(Rect::new(0, 0, 5, 3), &mut buf);
    for x in 5..60u16 {
        assert_eq!(
            buf[(x, 0)].symbol(),
            " ",
            "cell x{x} is outside the 5-wide box and must be untouched"
        );
    }
    assert_eq!(
        buf[(4, 0)].symbol(),
        "┐",
        "the box keeps its own right corner"
    );
}

#[test]
fn a_control_character_title_never_reaches_a_cell() {
    // A newline measures 1 cell, so it passes every bound, and ratatui flushes the byte:
    // the terminal drops a line for each and the whole frame desynchronises.
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
    let title = "\n".repeat(30);
    Boxed {
        title: &title,
        flag: None,
        focused: false,
    }
    .render(Rect::new(0, 0, 20, 3), &mut buf);
    for x in 0..20u16 {
        let s = buf[(x, 0)].symbol();
        assert!(
            !s.chars().any(char::is_control),
            "cell x{x} holds control character {s:?}"
        );
    }
}

#[test]
fn put_within_clips_at_the_boundary_it_is_given() {
    // The second layer, independent of sanitizing: even handed text that fits no budget,
    // a box's own right edge stops the write. A wide grapheme straddling the edge is
    // dropped whole rather than half-drawn.
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
    let end = put_within(&mut buf, 2, 0, 6, "abcdefghij", Style::default());
    assert_eq!(end, 7, "stops one past the inclusive boundary");
    assert_eq!(buf[(6, 0)].symbol(), "e");
    for x in 7..20u16 {
        assert_eq!(buf[(x, 0)].symbol(), " ", "cell x{x} is past the boundary");
    }

    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
    put_within(&mut buf, 2, 0, 3, "漢字", Style::default());
    assert_eq!(buf[(2, 0)].symbol(), "漢");
    assert_eq!(
        buf[(4, 0)].symbol(),
        " ",
        "the second wide grapheme does not fit and is dropped"
    );
}

#[test]
fn put_does_not_panic_on_a_zero_width_buffer() {
    // `put` computed its edge as `x + width - 1`, which underflows when width is 0. A client
    // reports its own cols and nothing clamps them, so a zero-width buffer is reachable.
    let mut buf = Buffer::empty(Rect::new(0, 0, 0, 1));
    let end = put(&mut buf, 0, 0, "abc", Style::default());
    assert_eq!(end, 0, "nothing is drawn and nothing panics");
}

#[test]
fn put_itself_refuses_control_and_zero_width_graphemes() {
    // Sanitizing only inside Boxed protected the pane box alone. Task 15 calls `put`
    // directly for the tab name, the prompt input and a path-derived location, and each
    // reproduces the original two escapes. The primitive has to be safe by itself.
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
    put(&mut buf, 0, 0, "a\nb", Style::default());
    for x in 0..10u16 {
        assert!(
            !buf[(x, 0)].symbol().chars().any(char::is_control),
            "cell x{x} holds a control character"
        );
    }

    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
    let end = put(&mut buf, 0, 0, &"\u{200b}".repeat(20), Style::default());
    assert_eq!(end, 0, "zero-width graphemes consume no cells");
}

#[test]
fn a_title_that_sanitizes_to_nothing_leaves_the_border_unbroken() {
    // A title of only zero-width graphemes becomes empty, and the " {title} " pads then
    // blanked two rule cells, printing `┌  ────────┐`.
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    Boxed {
        title: "\u{200b}\u{200b}",
        flag: None,
        focused: false,
    }
    .render(Rect::new(0, 0, 12, 3), &mut buf);
    assert_eq!(row(&buf, 0), "┌──────────┐");
}

/// A model with one project, one workspace, one tab and one pane, plus the clients given.
fn model_with_clients(
    sizes: &[(&str, Size)],
) -> (domux_core::model::Model, domux_core::ids::TabId) {
    use domux_core::model::{ClientView, Focus, Model};
    let mut model = Model::new(7);
    let (_, ws, _) = model
        .add_folder_project(std::path::PathBuf::from("/tmp/proj"))
        .unwrap();
    let (tab, pane, _) = model
        .create_tab(&ws, std::path::PathBuf::from("/tmp/proj"))
        .unwrap();
    for (id, size) in sizes {
        model.attach_client(ClientView {
            id: domux_core::ids::ClientId((*id).into()),
            size: *size,
            caps: Default::default(),
            workspace: ws.clone(),
            tab: tab.clone(),
            focus: Focus::Pane(pane.clone()),
            sidebar_open: false,
            sidebar_forced: false,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            agents_cursor: None,
            filtering: false,
            input: domux_core::model::TextInput::new(""),
            overlay_under: None,
            pill: None,
        });
    }
    (model, tab)
}

/// Task 15 is where a drawn area stops being written by hand and starts being computed, and
/// what it is computed from is a client's own reported size, which nothing clamps. Neither
/// `Boxed::render` nor `render_grid` checks its area against the buffer, so an area one
/// column too wide panics rather than clips.
///
/// The boundary is a client on the tab that is larger than the buffer being drawn: the pane
/// boxes take the smallest client's size, and a view the model does not hold - which
/// `compose` accepts, since it is public and takes the view by reference - would otherwise
/// take that larger client's size into a buffer its own size. Every cell must land inside.
#[test]
fn a_larger_client_on_the_tab_never_pushes_a_box_past_this_client_s_buffer() {
    use domux_core::model::ClientView;
    use domux_server::render::{compose, RenderInput};
    use std::collections::HashMap;

    let (model, tab) = model_with_clients(&[(
        "c_big",
        Size {
            cols: 200,
            rows: 60,
        },
    )]);
    // Deliberately not attached: the view is the one being drawn, and the model holds only
    // the larger client.
    let view = ClientView {
        size: Size { cols: 40, rows: 10 },
        ..model
            .client(&domux_core::ids::ClientId("c_big".into()))
            .unwrap()
            .clone()
    };
    assert_eq!(view.tab, tab);
    let panes = HashMap::new();
    let agents = domux_server::render::agents_box::AgentsView::empty(chrono::Local::now());
    let (buffer, _) = compose(&RenderInput {
        model: &model,
        facts: &domux_server::facts::FactRegistry::new(),
        panes: &panes,
        agents: &agents,
        view: &view,
        keymap: &domux_core::keymap::Keymap::defaults(),
        now: chrono::Local::now(),
        config_error: None,
        hint: None,
        notes: &[],
    });
    assert_eq!(buffer.area, Rect::new(0, 0, 40, 10));
    assert_eq!(
        row(&buffer, 1).chars().next(),
        Some('┌'),
        "the box still starts at the workpanel's left edge"
    );
    assert!(
        row(&buffer, 1).ends_with('┐'),
        "and its right border lands on the last column of this client's own screen: {:?}",
        row(&buffer, 1)
    );
}

/// The mirror of the case above: the smallest client is smaller than this one, so the box
/// stops short and the rest of the larger screen stays blank (tmux's rule).
#[test]
fn a_smaller_client_on_the_tab_shortens_the_box_and_leaves_the_rest_blank() {
    use domux_server::render::{compose, RenderInput};
    use std::collections::HashMap;

    let (model, _) = model_with_clients(&[
        ("c_big", Size { cols: 60, rows: 20 }),
        ("c_small", Size { cols: 40, rows: 10 }),
    ]);
    let view = model
        .client(&domux_core::ids::ClientId("c_big".into()))
        .unwrap()
        .clone();
    let panes = HashMap::new();
    let agents = domux_server::render::agents_box::AgentsView::empty(chrono::Local::now());
    let (buffer, _) = compose(&RenderInput {
        model: &model,
        facts: &domux_server::facts::FactRegistry::new(),
        panes: &panes,
        agents: &agents,
        view: &view,
        keymap: &domux_core::keymap::Keymap::defaults(),
        now: chrono::Local::now(),
        config_error: None,
        hint: None,
        notes: &[],
    });
    let top = row(&buffer, 1);
    assert_eq!(top.chars().nth(39), Some('┐'), "{top:?}");
    assert!(
        top[top.char_indices().nth(40).unwrap().0..]
            .chars()
            .all(|c| c == ' '),
        "everything past the smallest client's width is blank: {top:?}"
    );
}

/// The tab row's background reaches the cell under a wide grapheme's second half.
///
/// `boxed::put_within` patches a cell's style, so a run with no background of its own keeps
/// whatever the row was filled with - except for a wide grapheme, where it calls
/// `Cell::reset()` on the trailing cell first and clears the fill. That is why the row's
/// background is a parameter of `TabRow::draw` rather than left to the patch.
///
/// Asserted on the composed buffer, which is the general statement and not a fallback: the
/// renderer's promise is that the row it drew has one background all the way across, and
/// that is true whatever the transport does with it.
///
/// The hole does reach a client, by a path that is worth naming because two earlier attempts
/// at this comment named the wrong one. It is not the diff: `Buffer::diff` never yields a
/// wide grapheme's trailing cell, whether or not the cell changed. It is
/// `ClientConn::take_frame`, which has two branches - when `needs_full` is set it sends
/// every cell of the buffer rather than a diff, and that is the first frame after an attach,
/// every resize, and every frame dropped because the channel was full.
#[test]
fn a_wide_grapheme_in_a_tab_name_leaves_no_hole_in_the_top_bar() {
    use domux_core::model::ClientView;
    use domux_server::render::{compose, RenderInput};
    use std::collections::HashMap;

    // Wide enough that the tab row keeps both cells: at 40 columns the clock takes most of
    // the row and the tab that is not current is elided away, taking the wide name with it.
    let (mut model, tab) = model_with_clients(&[(
        "c_one",
        Size {
            cols: 120,
            rows: 24,
        },
    )]);
    // A second tab, so tab 1 is not the current one and its cell takes the row's background
    // rather than the accent fill, which sets its own and would cover the hole.
    let ws = model
        .client(&domux_core::ids::ClientId("c_one".into()))
        .unwrap()
        .workspace
        .clone();
    model
        .rename_tab(&tab, Some("日".into()))
        .expect("tab 1 takes a wide name");
    let (second, _, _) = model
        .create_tab(&ws, std::path::PathBuf::from("/tmp/proj"))
        .unwrap();
    let view = ClientView {
        tab: second,
        ..model
            .client(&domux_core::ids::ClientId("c_one".into()))
            .unwrap()
            .clone()
    };
    let panes = HashMap::new();
    let agents = domux_server::render::agents_box::AgentsView::empty(chrono::Local::now());
    let (buffer, _) = compose(&RenderInput {
        model: &model,
        panes: &panes,
        agents: &agents,
        view: &view,
        keymap: &domux_core::keymap::Keymap::defaults(),
        facts: &domux_server::facts::FactRegistry::new(),
        now: chrono::Local::now(),
        config_error: None,
        hint: None,
        notes: &[],
    });
    let mantle = ratatui::style::Color::Rgb(0x18, 0x18, 0x25);
    let wide = (0..120u16)
        .find(|x| buffer[(*x, 0u16)].symbol() == "日")
        .expect("the wide tab name is on the bar");
    assert_eq!(
        buffer[(wide, 0u16)].bg,
        mantle,
        "the cell the glyph starts in"
    );
    assert_eq!(
        buffer[(wide + 1, 0u16)].bg,
        mantle,
        "and the cell under its second half, which `put_within` had reset"
    );
}
