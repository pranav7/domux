//! alacritty_terminal behind the `Emulator` trait.

mod keys;

use crate::emulator::{Emulator, EmulatorConfig};
use crate::key::KeyEvent;
use crate::types::{Attrs, Color, Cursor, CursorShape, Grid, Rgb, Size};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AnsiColor, CursorShape as AnsiCursorShape, NamedColor, Processor, Rgb as AnsiRgb,
};
use alacritty_terminal::Term;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
struct Dims {
    cols: usize,
    rows: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Receives the terminal's events. `PtyWrite` carries replies the inner program is owed.
/// Color requests ask us to format a reply for a palette color we choose here.
#[derive(Clone)]
struct Listener {
    responses: Arc<Mutex<Vec<u8>>>,
    default_fg: Rgb,
    default_bg: Rgb,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => self
                .responses
                .lock()
                .unwrap()
                .extend_from_slice(text.as_bytes()),
            Event::ColorRequest(index, format) => {
                // Indices follow alacritty's palette: 256 is the default foreground, 257 the
                // default background. Other indices resolve to the standard 256-color palette
                // and are answered only for those two here; M1 forwards the real palette.
                let rgb = match index {
                    256 => Some(self.default_fg),
                    257 => Some(self.default_bg),
                    _ => None,
                };
                if let Some(c) = rgb {
                    let reply = format(AnsiRgb {
                        r: c.r,
                        g: c.g,
                        b: c.b,
                    });
                    self.responses
                        .lock()
                        .unwrap()
                        .extend_from_slice(reply.as_bytes());
                }
            }
            _ => {}
        }
    }
}

pub struct AlacrittyEmulator {
    term: Term<Listener>,
    parser: Processor,
    responses: Arc<Mutex<Vec<u8>>>,
    size: Size,
}

impl AlacrittyEmulator {
    pub fn new(config: EmulatorConfig) -> Self {
        let responses = Arc::new(Mutex::new(Vec::new()));
        let listener = Listener {
            responses: responses.clone(),
            default_fg: config.default_fg,
            default_bg: config.default_bg,
        };
        let term_config = Config {
            scrolling_history: config.scrollback_lines,
            // The kitty keyboard protocol is off by default in alacritty_terminal; domux
            // needs it so a client can distinguish keys like shift+enter.
            kitty_keyboard: true,
            ..Config::default()
        };
        let dims = Dims {
            cols: config.size.cols as usize,
            rows: config.size.rows as usize,
        };
        AlacrittyEmulator {
            term: Term::new(term_config, &dims, listener),
            parser: Processor::new(),
            responses,
            size: config.size,
        }
    }

    pub(crate) fn mode(&self) -> TermMode {
        *self.term.mode()
    }
}

impl Emulator for AlacrittyEmulator {
    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn take_responses(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.responses.lock().unwrap());
    }

    fn resize(&mut self, size: Size) {
        self.term.resize(Dims {
            cols: size.cols as usize,
            rows: size.rows as usize,
        });
        self.size = size;
    }

    fn size(&self) -> Size {
        self.size
    }

    fn snapshot_grid(&mut self, out: &mut Grid) {
        out.resize(self.size);
        let grid = self.term.grid();
        for r in 0..self.size.rows {
            let row = &grid[Line(r as i32)];
            for c in 0..self.size.cols {
                let src = &row[Column(c as usize)];
                let dst = out.cell_mut(r, c);
                dst.text.clear();
                if src
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    dst.width = 0;
                } else {
                    dst.width = if src.flags.contains(Flags::WIDE_CHAR) {
                        2
                    } else {
                        1
                    };
                    dst.text.push(src.c);
                    if let Some(extra) = src.zerowidth() {
                        for ch in extra {
                            dst.text.push(*ch);
                        }
                    }
                }
                dst.fg = color_from(src.fg);
                dst.bg = color_from(src.bg);
                dst.underline_color = src.underline_color().map(color_from);
                dst.attrs = attrs_from(src.flags);
            }
        }
    }

    fn cursor(&self) -> Cursor {
        let point: Point = self.term.grid().cursor.point;
        let style = self.term.cursor_style();
        let shape = match style.shape {
            AnsiCursorShape::Underline => CursorShape::Underline,
            AnsiCursorShape::Beam => CursorShape::Bar,
            _ => CursorShape::Block,
        };
        Cursor {
            row: point.line.0.max(0) as u16,
            col: point.column.0 as u16,
            visible: self.term.mode().contains(TermMode::SHOW_CURSOR),
            shape,
            blink: style.blinking,
        }
    }

    fn encode_key(&mut self, key: &KeyEvent, out: &mut Vec<u8>) {
        keys::encode(self.mode(), key, out);
    }

    fn encode_paste(&self, text: &str, out: &mut Vec<u8>) {
        let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
        crate::emulator::wrap_paste(text, bracketed, out);
    }
}

fn color_from(c: AnsiColor) -> Color {
    match c {
        AnsiColor::Spec(rgb) => Color::Rgb(Rgb {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
        }),
        AnsiColor::Indexed(i) => Color::Indexed(i),
        AnsiColor::Named(named) => match named {
            NamedColor::Foreground | NamedColor::Background | NamedColor::Cursor => Color::Default,
            NamedColor::Black => Color::Indexed(0),
            NamedColor::Red => Color::Indexed(1),
            NamedColor::Green => Color::Indexed(2),
            NamedColor::Yellow => Color::Indexed(3),
            NamedColor::Blue => Color::Indexed(4),
            NamedColor::Magenta => Color::Indexed(5),
            NamedColor::Cyan => Color::Indexed(6),
            NamedColor::White => Color::Indexed(7),
            NamedColor::BrightBlack => Color::Indexed(8),
            NamedColor::BrightRed => Color::Indexed(9),
            NamedColor::BrightGreen => Color::Indexed(10),
            NamedColor::BrightYellow => Color::Indexed(11),
            NamedColor::BrightBlue => Color::Indexed(12),
            NamedColor::BrightMagenta => Color::Indexed(13),
            NamedColor::BrightCyan => Color::Indexed(14),
            NamedColor::BrightWhite => Color::Indexed(15),
            // Dim and bright-foreground variants only appear through alacritty's own
            // renderer; the DIM attribute carries that information here.
            _ => Color::Default,
        },
    }
}

fn attrs_from(flags: Flags) -> Attrs {
    let mut a = Attrs::empty();
    let map = [
        (Flags::BOLD, Attrs::BOLD),
        (Flags::DIM, Attrs::DIM),
        (Flags::ITALIC, Attrs::ITALIC),
        (Flags::UNDERLINE, Attrs::UNDERLINE),
        (Flags::DOUBLE_UNDERLINE, Attrs::DOUBLE_UNDERLINE),
        (Flags::UNDERCURL, Attrs::CURLY_UNDERLINE),
        (Flags::DOTTED_UNDERLINE, Attrs::DOTTED_UNDERLINE),
        (Flags::DASHED_UNDERLINE, Attrs::DASHED_UNDERLINE),
        (Flags::INVERSE, Attrs::INVERSE),
        (Flags::HIDDEN, Attrs::INVISIBLE),
        (Flags::STRIKEOUT, Attrs::STRIKETHROUGH),
    ];
    for (flag, attr) in map {
        if flags.contains(flag) {
            a |= attr;
        }
    }
    a
}
