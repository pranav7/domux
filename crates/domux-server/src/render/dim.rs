//! Greys the panes that do not have the focus, so the one that does reads at full strength
//! (MUX-60, decision record 0064).
//!
//! A terminal has no opacity, so each cell's colours are mixed towards the terminal's own
//! background by `[terminal] dim_unfocused` percent, which is what a pane laid over that
//! background at less than full opacity looks like. The colours come from the terminal's
//! answers at attach. A cell whose colour the answers do not give, because the terminal did
//! not answer, is drawn faint instead, which is the nearest thing a terminal offers without
//! knowing the colour.

use domux_core::theme::{mix, TerminalColors};
use domux_term::Rgb;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// Mixes every cell of `area` `percent` of the way towards the terminal's background. 0 leaves
/// the cells as they are.
pub fn dim(buf: &mut Buffer, area: Rect, percent: u8, colors: &TerminalColors) {
    let percent = percent.min(100);
    if percent == 0 {
        return;
    }
    let right = area.right().min(buf.area.right());
    let bottom = area.bottom().min(buf.area.bottom());
    for y in area.y..bottom {
        for x in area.x..right {
            let cell = &mut buf[(x, y)];
            // A blank on the terminal's own background draws nothing a colour could change,
            // so it is left as it is rather than sent again with colours nobody sees.
            let shows = Modifier::REVERSED | Modifier::UNDERLINED | Modifier::CROSSED_OUT;
            if cell.symbol() == " " && cell.bg == Color::Reset && !cell.modifier.intersects(shows) {
                continue;
            }
            let Some(ground) = colors.bg else {
                cell.modifier.insert(Modifier::DIM);
                continue;
            };
            match rgb_of(cell.fg, colors.fg, colors) {
                Some(fg) => cell.fg = toward(fg, ground, percent),
                None => cell.modifier.insert(Modifier::DIM),
            }
            // A cell on the terminal's own background is already the colour the mix goes
            // towards, so it stays the terminal's default and follows it.
            if cell.bg != Color::Reset {
                if let Some(bg) = rgb_of(cell.bg, colors.bg, colors) {
                    cell.bg = toward(bg, ground, percent);
                }
            }
            if let Some(ul) = rgb_of(cell.underline_color, colors.fg, colors) {
                if cell.underline_color != Color::Reset {
                    cell.underline_color = toward(ul, ground, percent);
                }
            }
        }
    }
}

fn toward(from: Rgb, ground: Rgb, percent: u8) -> Color {
    let c = mix(from, ground, percent, 100);
    Color::Rgb(c.r, c.g, c.b)
}

/// The colour a cell asks for, as a value: `default` is the terminal's default for that
/// side, a palette slot under 16 is the terminal's answer for it, and the slots from 16 are
/// the fixed cube and greys every terminal shares.
fn rgb_of(color: Color, default: Option<Rgb>, colors: &TerminalColors) -> Option<Rgb> {
    let rgb = |r, g, b| Some(Rgb { r, g, b });
    match color {
        Color::Reset => default,
        Color::Rgb(r, g, b) => rgb(r, g, b),
        Color::Indexed(i) => indexed(i, colors),
        Color::Black => colors.palette[0],
        Color::Red => colors.palette[1],
        Color::Green => colors.palette[2],
        Color::Yellow => colors.palette[3],
        Color::Blue => colors.palette[4],
        Color::Magenta => colors.palette[5],
        Color::Cyan => colors.palette[6],
        Color::Gray => colors.palette[7],
        Color::DarkGray => colors.palette[8],
        Color::LightRed => colors.palette[9],
        Color::LightGreen => colors.palette[10],
        Color::LightYellow => colors.palette[11],
        Color::LightBlue => colors.palette[12],
        Color::LightMagenta => colors.palette[13],
        Color::LightCyan => colors.palette[14],
        Color::White => colors.palette[15],
    }
}

fn indexed(i: u8, colors: &TerminalColors) -> Option<Rgb> {
    match i {
        0..=15 => colors.palette[usize::from(i)],
        16..=231 => {
            let i = i - 16;
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            Some(Rgb {
                r: level(i / 36),
                g: level((i / 6) % 6),
                b: level(i % 6),
            })
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            Some(Rgb { r: v, g: v, b: v })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn answers() -> TerminalColors {
        let mut palette = [None; 16];
        palette[1] = Some(Rgb { r: 200, g: 0, b: 0 });
        TerminalColors {
            fg: Some(Rgb {
                r: 200,
                g: 200,
                b: 200,
            }),
            bg: Some(Rgb { r: 0, g: 0, b: 0 }),
            palette,
        }
    }

    fn one_cell(style: Style) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)].set_symbol("x").set_style(style);
        buf
    }

    #[test]
    fn a_default_foreground_mixes_towards_the_background() {
        let mut buf = one_cell(Style::default());
        dim(&mut buf, Rect::new(0, 0, 1, 1), 25, &answers());
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(150, 150, 150));
        assert_eq!(
            buf[(0, 0)].bg,
            Color::Reset,
            "the terminal's own background stays its default"
        );
        assert!(!buf[(0, 0)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn a_palette_slot_mixes_from_the_terminals_answer_for_it() {
        let mut buf = one_cell(Style::default().fg(Color::Indexed(1)).bg(Color::Indexed(1)));
        dim(&mut buf, Rect::new(0, 0, 1, 1), 50, &answers());
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(100, 0, 0));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(100, 0, 0));
    }

    #[test]
    fn the_cube_and_the_greys_need_no_answer() {
        assert_eq!(indexed(16, &answers()), Some(Rgb { r: 0, g: 0, b: 0 }));
        assert_eq!(
            indexed(231, &answers()),
            Some(Rgb {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert_eq!(indexed(196, &answers()), Some(Rgb { r: 255, g: 0, b: 0 }));
        assert_eq!(indexed(232, &answers()), Some(Rgb { r: 8, g: 8, b: 8 }));
    }

    #[test]
    fn a_colour_the_terminal_did_not_answer_is_drawn_faint() {
        let mut buf = one_cell(Style::default().fg(Color::Indexed(2)));
        dim(&mut buf, Rect::new(0, 0, 1, 1), 30, &answers());
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(2));
        assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
        let mut buf = one_cell(Style::default());
        dim(
            &mut buf,
            Rect::new(0, 0, 1, 1),
            30,
            &TerminalColors::default(),
        );
        assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn a_blank_on_the_terminals_background_is_left_alone() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let before = buf.clone();
        dim(&mut buf, Rect::new(0, 0, 1, 1), 30, &answers());
        assert_eq!(buf, before);
        dim(
            &mut buf,
            Rect::new(0, 0, 1, 1),
            30,
            &TerminalColors::default(),
        );
        assert_eq!(buf, before, "and is not drawn faint either");
        let mut buf = one_cell(Style::default().add_modifier(Modifier::REVERSED));
        buf[(0, 0)].set_symbol(" ");
        dim(&mut buf, Rect::new(0, 0, 1, 1), 50, &answers());
        assert_eq!(
            buf[(0, 0)].fg,
            Color::Rgb(100, 100, 100),
            "a reversed blank shows its foreground"
        );
    }

    #[test]
    fn zero_percent_leaves_the_cells_alone() {
        let mut buf = one_cell(Style::default());
        let before = buf.clone();
        dim(&mut buf, Rect::new(0, 0, 1, 1), 0, &answers());
        assert_eq!(buf, before);
    }
}
