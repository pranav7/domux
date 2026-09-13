//! One attached client: its capabilities and the diff to the last queued frame.

use domux_core::ids::ClientId;
use domux_core::proto::{Capabilities, CellUpdate, CursorState, FrameDiff, ServerMsg, WireColor};
use domux_core::theme::{Desktop, TerminalColors, Theme, Themes};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tokio::sync::mpsc;

/// What a hint is, which is what decides when it goes away.
///
/// The field holds messages with different lifetimes, and telling them apart by their text
/// only works while the text can be reproduced: the shell-failure notice names the
/// configured shell, so a `config.reload` that changes `terminal.shell` used to leave a
/// stored notice no generated string matched any more, and the next key cleared a notice
/// that was still true. The kind is the identity; the text is only what the reader sees.
///
/// A config error is deliberately not one of these kinds. It is server state - the config
/// that is in force and the file that was rejected - and it reaches the bar as
/// `RenderInput.config_error` (ruled 2026-09-07). A hint is losable: it lives on one
/// connection, another hint replaces it, and `clear_action_hint` runs on every key. A config
/// error has to stand until the file parses, which is not something the reader can type their
/// way out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintKind {
    /// The answer to one key: a failed action, a clipboard that could not be written. It
    /// stands until the next key and no longer.
    Action,
    /// The respawn guard's notice. True until a shell survives or the config is reloaded.
    ShellFailure,
}

/// One line for the clock's place, and what makes it go away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub kind: HintKind,
    pub text: String,
}

impl Hint {
    pub fn action(text: impl Into<String>) -> Hint {
        Hint {
            kind: HintKind::Action,
            text: text.into(),
        }
    }

    pub fn shell_failure(text: impl Into<String>) -> Hint {
        Hint {
            kind: HintKind::ShellFailure,
            text: text.into(),
        }
    }
}

pub struct ClientConn {
    pub id: ClientId,
    pub tx: mpsc::Sender<ServerMsg>,
    pub caps: Capabilities,
    pub previous: Buffer,
    /// The next frame carries every cell: first frame, after a resize, after a reattach.
    pub needs_full: bool,
    pub last_cursor: Option<CursorState>,
    /// A one-line notice for the clock's place, such as a clipboard failure.
    pub hint: Option<Hint>,
    /// Whether the client's theme reads its terminal, when the message that says so found the
    /// channel full. It goes out ahead of the next frame.
    follow_unsent: Option<bool>,
    /// This client's theme, painted for its terminal's colours.
    pub theme: ThemeCache,
}

/// One client's painted theme, with what it was painted from. It is painted again only when
/// one of those changed: a reload, a colours message, or a new desktop.
#[derive(Debug, Default)]
pub struct ThemeCache {
    painted: Option<(Themes, TerminalColors, Desktop, Theme)>,
    /// How many times it painted, for the tests.
    #[cfg(test)]
    paints: usize,
}

impl ThemeCache {
    /// The theme `themes` paints for these colours on this desktop.
    pub fn get(&mut self, themes: &Themes, colors: &TerminalColors, desktop: Desktop) -> &Theme {
        let fresh = matches!(
            &self.painted,
            Some((t, c, d, _)) if t == themes && c == colors && *d == desktop
        );
        if !fresh {
            self.painted = None;
        }
        let (_, _, _, theme) = self.painted.get_or_insert_with(|| {
            #[cfg(test)]
            {
                self.paints += 1;
            }
            (
                themes.clone(),
                colors.clone(),
                desktop,
                themes.paint(colors, desktop),
            )
        });
        theme
    }
}

impl ClientConn {
    pub fn new(
        id: ClientId,
        tx: mpsc::Sender<ServerMsg>,
        caps: Capabilities,
        cols: u16,
        rows: u16,
    ) -> ClientConn {
        let area = Rect::new(0, 0, cols, rows);
        ClientConn {
            id,
            tx,
            caps,
            previous: Buffer::empty(area),
            needs_full: true,
            last_cursor: None,
            hint: None,
            follow_unsent: None,
            theme: ThemeCache::default(),
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let area = Rect::new(0, 0, cols, rows);
        self.previous = Buffer::empty(area);
        self.needs_full = true;
    }

    /// Queues one composed frame. The only caller of `take_frame` in the server, so the
    /// state that call advances to and the message that carries it cannot disagree: a full
    /// channel means the client never received that state, and the flag set here makes the
    /// next frame replace the whole screen rather than diff against a frame it never saw.
    /// Tells the client whether its theme reads its terminal. A full channel keeps the message
    /// for the next frame, since a client never told keeps following, or not, until it attaches
    /// again.
    pub fn send_follow(&mut self, follow: bool) {
        self.follow_unsent = Some(follow);
        self.flush_follow();
    }

    fn flush_follow(&mut self) {
        if let Some(follow) = self.follow_unsent {
            if self.tx.try_send(ServerMsg::FollowColors(follow)).is_ok() {
                self.follow_unsent = None;
            }
        }
    }

    pub fn queue_frame(&mut self, composed: Buffer, cursor: Option<CursorState>) {
        self.flush_follow();
        if let Some(diff) = self.take_frame(composed, cursor) {
            if self.tx.try_send(ServerMsg::Frame(diff)).is_err() {
                self.needs_full = true;
            }
        }
    }

    /// Replaces the buffer with a composed frame and returns what changed, or `None` when
    /// nothing did. The caller sends the diff.
    pub fn take_frame(
        &mut self,
        composed: Buffer,
        cursor: Option<CursorState>,
    ) -> Option<FrameDiff> {
        let full = self.needs_full;
        let cells: Vec<CellUpdate> = if full {
            composed
                .content
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let (x, y) = composed.pos_of(i);
                    to_update(x, y, c)
                })
                .collect()
        } else {
            self.previous
                .diff(&composed)
                .into_iter()
                .map(|(x, y, c)| to_update(x, y, c))
                .collect()
        };
        let cursor_changed = cursor != self.last_cursor;
        self.previous = composed;
        self.needs_full = false;
        if cells.is_empty() && !cursor_changed && !full {
            return None;
        }
        self.last_cursor = cursor.clone();
        Some(FrameDiff {
            full,
            cols: self.previous.area.width,
            rows: self.previous.area.height,
            cells,
            cursor,
        })
    }
}

fn to_update(x: u16, y: u16, c: &ratatui::buffer::Cell) -> CellUpdate {
    CellUpdate {
        x,
        y,
        symbol: c.symbol().to_string(),
        fg: wire(c.fg),
        bg: wire(c.bg),
        underline: if c.underline_color == Color::Reset {
            None
        } else {
            Some(wire(c.underline_color))
        },
        modifiers: c.modifier.bits(),
    }
}

fn wire(c: Color) -> WireColor {
    match c {
        Color::Rgb(r, g, b) => WireColor::Rgb(r, g, b),
        Color::Indexed(i) => WireColor::Indexed(i),
        Color::Reset => WireColor::Reset,
        named => WireColor::Indexed(wire_index(named)),
    }
}

/// The ANSI index a ratatui named colour stands for, 0 to 15. Named colours have no place
/// on the wire, so this is the one mapping: `wire` sends it and the harness reads a frame
/// back through it, and a client rendering `Indexed(1)` paints the cell a client rendering
/// `Red` would.
///
/// `Rgb`, `Indexed` and `Reset` are not named colours and have no index; they answer 0,
/// which is why `wire` handles them before it asks.
pub fn wire_index(c: Color) -> u8 {
    match c {
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Rgb(..) | Color::Indexed(_) | Color::Reset => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_term::CursorShape;
    use ratatui::style::Style;

    fn conn() -> ClientConn {
        // The channel is never read here: these tests are about the diff, not the delivery.
        let (tx, _rx) = mpsc::channel(8);
        ClientConn::new(ClientId("c_0d77".into()), tx, Capabilities::default(), 3, 2)
    }

    fn buffer_with(x: u16, y: u16, symbol: &str) -> Buffer {
        let mut b = Buffer::empty(Rect::new(0, 0, 3, 2));
        b[(x, y)].set_symbol(symbol);
        b
    }

    #[test]
    fn the_first_frame_is_full_and_the_next_carries_only_the_changed_cells() {
        let mut c = conn();
        let first = c
            .take_frame(Buffer::empty(Rect::new(0, 0, 3, 2)), None)
            .expect("the first frame is always sent");
        assert!(first.full);
        assert_eq!((first.cols, first.rows), (3, 2));
        assert_eq!(first.cells.len(), 6, "every cell of a 3x2 screen");
        let second = c
            .take_frame(buffer_with(1, 1, "x"), None)
            .expect("one cell changed");
        assert!(!second.full);
        assert_eq!(second.cells.len(), 1);
        assert_eq!((second.cells[0].x, second.cells[0].y), (1, 1));
        assert_eq!(second.cells[0].symbol, "x");
    }

    #[test]
    fn a_frame_that_changed_nothing_is_not_sent() {
        let mut c = conn();
        c.take_frame(buffer_with(0, 0, "a"), None).unwrap();
        assert!(c.take_frame(buffer_with(0, 0, "a"), None).is_none());
    }

    #[test]
    fn a_cursor_that_moved_is_sent_even_when_no_cell_changed() {
        let mut c = conn();
        c.take_frame(buffer_with(0, 0, "a"), None).unwrap();
        let moved = c
            .take_frame(
                buffer_with(0, 0, "a"),
                Some(CursorState {
                    x: 2,
                    y: 1,
                    shape: CursorShape::Block,
                    blink: false,
                }),
            )
            .expect("the cursor moved");
        assert!(moved.cells.is_empty());
        assert_eq!(moved.cursor.map(|c| (c.x, c.y)), Some((2, 1)));
    }

    /// A resize gives the client a screen it has never drawn, so the next frame carries all
    /// of it. A diff against the old buffer would leave the new area unpainted.
    #[test]
    fn a_resize_makes_the_next_frame_full() {
        let mut c = conn();
        c.take_frame(Buffer::empty(Rect::new(0, 0, 3, 2)), None)
            .unwrap();
        c.resize(4, 3);
        let after = c
            .take_frame(Buffer::empty(Rect::new(0, 0, 4, 3)), None)
            .expect("a full frame follows a resize");
        assert!(after.full);
        assert_eq!((after.cols, after.rows), (4, 3));
        assert_eq!(after.cells.len(), 12);
    }

    /// `take_frame` advances `previous` whether or not the frame reaches the client, so a
    /// send that fails would otherwise leave the two sides diffing against different
    /// screens for good. `queue_frame` owns both halves, so a drop is repaired.
    #[tokio::test]
    async fn a_dropped_frame_makes_the_next_queued_frame_full() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut c = ClientConn::new(ClientId("c_0d77".into()), tx, Capabilities::default(), 3, 2);
        c.tx.try_send(ServerMsg::Bell).unwrap();
        c.queue_frame(buffer_with(0, 0, "a"), None);
        assert!(matches!(rx.recv().await, Some(ServerMsg::Bell)));
        c.queue_frame(buffer_with(0, 0, "a"), None);
        let queued = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("the next frame is queued before the deadline")
            .expect("the frame channel stays open");
        match queued {
            ServerMsg::Frame(frame) => assert!(frame.full),
            other => panic!("{other:?}"),
        }
    }

    /// A client whose channel is full when its theme starts or stops reading the terminal is
    /// told on the next frame queued for it, rather than never.
    #[tokio::test]
    async fn a_follow_that_found_the_channel_full_is_sent_with_the_next_frame() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut c = ClientConn::new(ClientId("c_0d77".into()), tx, Capabilities::default(), 3, 2);
        c.tx.try_send(ServerMsg::Bell).unwrap();
        c.send_follow(true);
        assert!(matches!(rx.recv().await, Some(ServerMsg::Bell)));
        c.queue_frame(buffer_with(0, 0, "a"), None);
        assert!(matches!(
            rx.recv().await,
            Some(ServerMsg::FollowColors(true))
        ));
        // Sent once: the next frame carries no second copy.
        c.queue_frame(buffer_with(0, 0, "b"), None);
        assert!(matches!(rx.recv().await, Some(ServerMsg::Frame(_))));
    }

    /// The theme is painted once for what it is painted from, not on every frame: the guards
    /// walk every role over every ground, and a working agent composes 14 frames a second.
    #[test]
    fn a_theme_is_painted_again_only_when_the_themes_the_colours_or_the_desktop_change() {
        use domux_core::theme::{Desktop, TerminalColors, ThemeChoice, Themes};
        let mut cache = ThemeCache::default();
        let terminal = Themes {
            choice: ThemeChoice::Builtin("terminal"),
            file: None,
        };
        let rgb = |r, g, b| domux_term::Rgb { r, g, b };
        let ristretto = TerminalColors {
            fg: Some(rgb(0xe6, 0xd9, 0xdb)),
            bg: Some(rgb(0x2c, 0x25, 0x25)),
            palette: [None; 16],
        };
        let first = cache.get(&terminal, &ristretto, Desktop::Unknown).clone();
        assert_eq!(first, terminal.paint(&ristretto, Desktop::Unknown));
        cache.get(&terminal, &ristretto, Desktop::Unknown);
        assert_eq!(cache.paints, 1);

        let latte = TerminalColors {
            fg: Some(rgb(0x4c, 0x4f, 0x69)),
            bg: Some(rgb(0xef, 0xf1, 0xf5)),
            palette: [None; 16],
        };
        assert_eq!(
            cache.get(&terminal, &latte, Desktop::Unknown),
            &terminal.paint(&latte, Desktop::Unknown)
        );
        assert_eq!(cache.paints, 2);

        let auto = Themes::default();
        assert_eq!(
            cache.get(&auto, &latte, Desktop::Unknown),
            &auto.paint(&latte, Desktop::Unknown)
        );
        assert_eq!(cache.paints, 3);
        assert_eq!(
            cache.get(&auto, &latte, Desktop::Omarchy),
            &auto.paint(&latte, Desktop::Omarchy)
        );
        assert_eq!(cache.paints, 4);
    }

    /// ratatui's named colours have no place on the wire, so each maps to the ANSI index it
    /// stands for. A client rendering `Indexed(1)` and a client rendering `Red` must paint
    /// the same cell.
    #[test]
    fn colours_reach_the_wire_as_rgb_indexed_or_reset() {
        let mut c = conn();
        let mut b = Buffer::empty(Rect::new(0, 0, 3, 2));
        b[(0, 0)].set_style(Style::default().fg(Color::Red).bg(Color::Rgb(1, 2, 3)));
        b[(1, 0)].set_style(Style::default().underline_color(Color::LightCyan));
        let frame = c.take_frame(b, None).unwrap();
        let cell = |x: u16, y: u16| {
            frame
                .cells
                .iter()
                .find(|c| c.x == x && c.y == y)
                .expect("cell")
        };
        assert_eq!(cell(0, 0).fg, WireColor::Indexed(1));
        assert_eq!(cell(0, 0).bg, WireColor::Rgb(1, 2, 3));
        assert_eq!(
            cell(0, 0).underline,
            None,
            "an unset underline colour is absent, not Reset"
        );
        assert_eq!(cell(1, 0).underline, Some(WireColor::Indexed(14)));
        assert_eq!(cell(2, 1).fg, WireColor::Reset);
    }
}
