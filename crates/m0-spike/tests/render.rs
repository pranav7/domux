use domux_term::{Attrs, Color, Cursor, Grid, Rgb, Size};
use m0_spike::render::{cursor_position, PaneWidget};
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::Terminal;

fn grid() -> Grid {
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
    g
}

#[test]
fn cells_map_to_symbols_styles_and_wide_spacers() {
    let backend = TestBackend::new(6, 2);
    let mut terminal = Terminal::new(backend).unwrap();
    let g = grid();
    terminal
        .draw(|f| {
            f.render_widget(
                PaneWidget {
                    grid: &g,
                    focused: true,
                    border: false,
                },
                f.area(),
            )
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    assert_eq!(buf[(0, 0)].symbol(), "a");
    assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(0, 0)].fg, ratatui::style::Color::Indexed(1));
    assert_eq!(buf[(1, 0)].symbol(), "漢");
    assert_eq!(
        buf[(2, 0)].symbol(),
        " ",
        "spacer cell is reset so the diff skips it"
    );
    assert_eq!(buf[(3, 0)].symbol(), "x");
    assert_eq!(buf[(3, 0)].bg, ratatui::style::Color::Rgb(1, 2, 3));
    assert_eq!(buf[(5, 1)].symbol(), " ");
}

#[test]
fn cursor_position_offsets_by_the_border_and_hides_when_invisible() {
    let area = Rect::new(10, 5, 20, 10);
    let c = Cursor {
        row: 2,
        col: 3,
        ..Cursor::default()
    };
    assert_eq!(
        cursor_position(area, true, &c),
        Some(Position { x: 14, y: 8 })
    );
    assert_eq!(
        cursor_position(area, false, &c),
        Some(Position { x: 13, y: 7 })
    );
    assert_eq!(
        cursor_position(
            area,
            false,
            &Cursor {
                visible: false,
                ..c
            }
        ),
        None
    );
}
