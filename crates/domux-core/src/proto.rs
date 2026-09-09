//! The attach protocol. Task 9 fills this file; the Model needs `Capabilities` first.

use crate::ids::ClientId;
use domux_term::{CursorShape, KeyEvent, MouseEvent, Rgb};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the client's outer terminal can do, negotiated at attach.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    pub truecolor: bool,
    pub kitty_keyboard: bool,
    pub hyperlinks: bool,
    /// The outer terminal accepts OSC 52 for the clipboard.
    pub osc52: bool,
    /// The outer terminal's default colours when it answered OSC 10 and 11.
    pub default_fg: Option<Rgb>,
    pub default_bg: Option<Rgb>,
}

/// Bumped when a message shape changes. The server refuses a client with another value.
/// 3 added `ClientMsg::Mouse` (decision 0014).
pub const PROTOCOL_VERSION: u32 = 3;
/// A frame larger than this is a bug or an attack, never a screen. The value is also what
/// keeps the two protocols on one socket apart: see `is_control_api_first_byte`.
pub const MAX_FRAME: u32 = 64 * 1024 * 1024;

/// The two constants are related, so hold the relation here rather than trusting a reader to
/// re-derive it. A valid frame's first byte is the top byte of a length no larger than
/// `MAX_FRAME`, and `is_control_api_first_byte` is sound only while that byte can never be
/// `{`. Raising `MAX_FRAME` to 0x7b000000 or above breaks the build instead of misrouting an
/// attach connection.
const _: () = assert!(
    MAX_FRAME.to_be_bytes()[0] < b'{',
    "MAX_FRAME is so large that a frame's first byte could be the control API's opening brace"
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// `CARGO_PKG_VERSION` of the client binary. Must equal the server's, so a stale server
    /// after an upgrade fails with an instruction instead of corrupt output.
    pub version: String,
    pub protocol: u32,
    pub cols: u16,
    pub rows: u16,
    pub caps: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello(Hello),
    Key(KeyEvent),
    /// A vertical scroll gesture at an outer-terminal cell. Positive lines move away from
    /// the live screen; negative lines move back towards it.
    Scroll {
        column: u16,
        row: u16,
        lines: i16,
    },
    /// A mouse button pressed, dragged or released at an outer-terminal cell. The event's own
    /// `row` and `col` are that cell: the server rebases them onto whatever they hit.
    Mouse {
        event: MouseEvent,
        /// Which press of a repeated click this is: 1, 2 for a double, 3 for a triple, and 1
        /// again after that. A drag and a release carry the count of the press they belong to.
        ///
        /// The client counts, for the reason it turns one wheel notch into a fixed number of
        /// lines: the timing is the outer terminal's, not the server's. It also keeps what the
        /// server does a function of the messages it was sent, so a double click is a test
        /// rather than two presses and a sleep.
        count: u8,
    },
    Paste(String),
    Resize {
        cols: u16,
        rows: u16,
    },
    /// The outer terminal gained or lost focus.
    Focus(bool),
    Detach,
    /// The client could not write the clipboard; the reason is shown as a hint.
    ClipboardFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireColor {
    Reset,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// One changed cell. `modifiers` are ratatui's `Modifier` bits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellUpdate {
    pub x: u16,
    pub y: u16,
    pub symbol: String,
    pub fg: WireColor,
    pub bg: WireColor,
    pub underline: Option<WireColor>,
    pub modifiers: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub shape: CursorShape,
    pub blink: bool,
}

/// The cells that changed since the previous frame. `full` means the client must clear
/// first (first frame, resize, or reattach). `cursor: None` hides the cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameDiff {
    pub full: bool,
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<CellUpdate>,
    pub cursor: Option<CursorState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    Welcome {
        client: ClientId,
        version: String,
    },
    Refused {
        reason: String,
    },
    Frame(FrameDiff),
    /// Text for the outer terminal's clipboard (copy mode's Enter).
    Clipboard(String),
    Bell,
    Detached {
        reason: String,
    },
}

/// The reason the server sends when the whole server is going away, rather than one view. Both
/// arrive as `ServerMsg::Detached`, so this string is the only thing that tells them apart, and
/// the server writing it and the client reading it have to agree letter for letter. It lives
/// here, next to the message that carries it, so the compiler holds that agreement rather than
/// a test on each side pinning its own copy.
pub const SERVER_STOPPED: &str = "the server stopped";

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("frame of {0} bytes exceeds the {MAX_FRAME} byte limit")]
    FrameTooLarge(u32),
    #[error("could not decode a message: {0}")]
    Decode(String),
}

/// The 4-byte big-endian length prefix for a body of `body_len` bytes, or `FrameTooLarge`
/// when no honest prefix exists: the decoder would refuse the frame anyway, and above
/// `u32::MAX` the length does not fit the prefix at all, so writing one would put a number on
/// the wire that is not the body's length and desynchronise the stream. Split out from
/// `encode` so the limit is testable without allocating a frame that size.
fn length_prefix(body_len: usize) -> Result<[u8; 4], ProtoError> {
    if body_len > MAX_FRAME as usize {
        // A body wider than the variant's `u32` cannot be reported exactly. `u32::MAX` reads
        // as "at least this large", and it is far above the limit either way.
        return Err(ProtoError::FrameTooLarge(
            u32::try_from(body_len).unwrap_or(u32::MAX),
        ));
    }
    Ok((body_len as u32).to_be_bytes())
}

/// A message as bytes: a 4-byte big-endian length, then bincode. Fails rather than emit a
/// frame the peer's `Decoder` would refuse.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtoError> {
    let body = bincode::serialize(msg).expect("serializable message");
    let prefix = length_prefix(body.len())?;
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&prefix);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Accumulates bytes and yields whole messages.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, or `Ok(None)` when the buffer does not hold one yet. Push
    /// more bytes and call again.
    ///
    /// A `ProtoError` is terminal for the stream: the caller must drop the connection. The
    /// offending bytes are deliberately left at the front of the buffer, so a caller that
    /// ignores the error gets the same error forever rather than silent progress. Draining
    /// past a bad frame would mean trusting the length that the failed decode just called
    /// into question, and resynchronising on a guess is worse than stopping.
    // Not `Iterator::next`: the message type is chosen per call, and decoding can fail.
    #[allow(clippy::should_implement_trait)]
    pub fn next<T: serde::de::DeserializeOwned>(&mut self) -> Result<Option<T>, ProtoError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        if len > MAX_FRAME {
            return Err(ProtoError::FrameTooLarge(len));
        }
        let end = 4 + len as usize;
        if self.buf.len() < end {
            return Ok(None);
        }
        let msg = bincode::deserialize::<T>(&self.buf[4..end])
            .map_err(|e| ProtoError::Decode(e.to_string()))?;
        self.buf.drain(..end);
        Ok(Some(msg))
    }
}

/// The control API is newline-delimited JSON, so its first byte is `{`. An attach frame
/// starts with a big-endian length, so its first byte is the top byte of a body length that
/// `MAX_FRAME` caps at 64 MiB, which is at most 0x04. `{` is 0x7b, so the two can never
/// collide. The const assertion next to `MAX_FRAME` enforces that relation.
pub fn is_control_api_first_byte(b: u8) -> bool {
    b == b'{'
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_term::{Key, KeyAction, KeyEvent, Mods};

    #[test]
    fn messages_round_trip_through_the_length_prefixed_frames() {
        let hello = ClientMsg::Hello(Hello {
            version: "2.0.0-alpha.0".into(),
            protocol: PROTOCOL_VERSION,
            cols: 120,
            rows: 40,
            caps: Capabilities {
                truecolor: true,
                ..Default::default()
            },
        });
        let key = ClientMsg::Key(KeyEvent {
            key: Key::Char('|'),
            mods: Mods::SHIFT,
            action: KeyAction::Press,
        });
        let scroll = ClientMsg::Scroll {
            column: 20,
            row: 8,
            lines: 3,
        };
        let mut bytes = encode(&hello).unwrap();
        bytes.extend(encode(&key).unwrap());
        bytes.extend(encode(&scroll).unwrap());
        let mut d = Decoder::default();
        // Feed one byte at a time to prove partial frames are buffered.
        let mut out: Vec<ClientMsg> = Vec::new();
        for b in bytes {
            d.push(&[b]);
            while let Some(m) = d.next::<ClientMsg>().unwrap() {
                out.push(m);
            }
        }
        assert_eq!(out, vec![hello, key, scroll]);
    }

    #[test]
    fn frame_diff_round_trips_with_styles_and_cursor() {
        let diff = FrameDiff {
            full: true,
            cols: 3,
            rows: 1,
            cells: vec![CellUpdate {
                x: 1,
                y: 0,
                symbol: "漢".into(),
                fg: WireColor::Rgb(0xcb, 0xa6, 0xf7),
                bg: WireColor::Reset,
                underline: Some(WireColor::Indexed(1)),
                modifiers: 0b101,
            }],
            cursor: Some(CursorState {
                x: 2,
                y: 0,
                shape: domux_term::CursorShape::Bar,
                blink: true,
            }),
        };
        let msg = ServerMsg::Frame(diff.clone());
        let mut d = Decoder::default();
        d.push(&encode(&msg).unwrap());
        assert_eq!(d.next::<ServerMsg>().unwrap(), Some(ServerMsg::Frame(diff)));
        assert_eq!(d.next::<ServerMsg>().unwrap(), None);
    }

    #[test]
    fn oversized_frames_are_rejected_before_allocation() {
        let mut d = Decoder::default();
        d.push(&(MAX_FRAME + 1).to_be_bytes());
        assert!(matches!(
            d.next::<ServerMsg>(),
            Err(ProtoError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn the_first_byte_tells_the_protocols_apart() {
        assert!(is_control_api_first_byte(b'{'));
        assert!(!is_control_api_first_byte(
            encode(&ClientMsg::Detach).unwrap()[0]
        ));
        // The hello is the frame the discrimination actually sees, so pin that one too.
        let hello = ClientMsg::Hello(Hello {
            version: "2.0.0-alpha.0".into(),
            protocol: PROTOCOL_VERSION,
            cols: 120,
            rows: 40,
            caps: Capabilities::default(),
        });
        assert!(!is_control_api_first_byte(encode(&hello).unwrap()[0]));
    }

    /// The framing is the contract between a client and a server that were built at
    /// different times, so pin the bytes themselves. A round trip is symmetric and stays
    /// green through a change to the byte order or to whether the length counts itself.
    #[test]
    fn a_frame_is_a_big_endian_length_then_the_bincode_body() {
        // `Detach` is variant 7, which bincode writes as a 4-byte body.
        assert_eq!(
            encode(&ClientMsg::Detach).unwrap(),
            vec![0, 0, 0, 4, 7, 0, 0, 0]
        );
    }

    /// 123 is `{`. With a little-endian length this frame's first byte was `{`, so a real
    /// attach connection was routed to the control API.
    #[test]
    fn a_123_byte_body_is_not_mistaken_for_the_control_api() {
        let frame = encode(&ClientMsg::Paste("x".repeat(111))).unwrap();
        assert_eq!(frame.len() - 4, 123, "the body must be exactly 123 bytes");
        assert!(!is_control_api_first_byte(frame[0]));
    }

    #[test]
    fn encoding_refuses_a_body_over_the_frame_limit() {
        // The peer would answer `FrameTooLarge` and, since that error is terminal, then stall
        // on it forever, so the send side has to refuse first.
        assert!(length_prefix(MAX_FRAME as usize).is_ok());
        assert!(matches!(
            length_prefix(MAX_FRAME as usize + 1),
            Err(ProtoError::FrameTooLarge(n)) if n == MAX_FRAME + 1
        ));
    }

    #[test]
    fn encode_refuses_a_body_over_the_frame_limit_rather_than_truncating_it() {
        // The two tests around this one pin `length_prefix`, but `encode` is the public
        // surface and the `?` that connects the two is the whole point of the check: an
        // `encode` that swallowed the error would write a length that is not the body's
        // length again, and every later frame on that stream would start in the wrong place.
        // One 64 MiB body is the cheapest way to hold `encode` itself to it.
        let over_the_limit = ClientMsg::Paste("x".repeat(MAX_FRAME as usize));
        assert!(matches!(
            encode(&over_the_limit),
            Err(ProtoError::FrameTooLarge(n)) if n > MAX_FRAME
        ));
    }

    #[test]
    fn a_body_wider_than_the_length_field_is_refused_not_truncated() {
        // 4 GiB + 1 truncates to a length of 1, which would put a number on the wire that is
        // not the body's length and desynchronise every later frame.
        assert!(matches!(
            length_prefix(1usize << 32 | 1),
            Err(ProtoError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn a_zero_length_frame_is_a_decode_error() {
        let mut d = Decoder::default();
        d.push(&0u32.to_be_bytes());
        assert!(matches!(d.next::<ClientMsg>(), Err(ProtoError::Decode(_))));
    }

    #[test]
    fn a_decode_error_repeats_so_a_stream_cannot_silently_resume() {
        let mut d = Decoder::default();
        // A well-formed 4-byte frame whose body is not a `ClientMsg`: 0xffffffff is no variant.
        d.push(&4u32.to_be_bytes());
        d.push(&[0xff, 0xff, 0xff, 0xff]);
        d.push(&encode(&ClientMsg::Detach).unwrap());
        assert!(matches!(d.next::<ClientMsg>(), Err(ProtoError::Decode(_))));
        // The bad bytes stay put, so the good frame behind them is never mistaken for
        // progress. The caller must drop the connection.
        assert!(matches!(d.next::<ClientMsg>(), Err(ProtoError::Decode(_))));
    }

    /// `Mods` is a `bitflags` type, so its serialized shape comes from the bitflags crate
    /// rather than from a derive in this repository, and a version bump there could change
    /// the wire format without any code here moving. A client and a server built at
    /// different times then talk past each other, so pin both shapes: the flag-name string
    /// a human-readable format writes, and the raw bits bincode puts on the socket.
    #[test]
    fn key_modifiers_keep_their_serialized_shape() {
        let mods = Mods::SHIFT | Mods::CTRL;
        assert_eq!(serde_json::to_string(&mods).unwrap(), r#""SHIFT | CTRL""#);
        assert_eq!(
            serde_json::from_str::<Mods>(r#""SHIFT | CTRL""#).unwrap(),
            mods
        );
        assert_eq!(serde_json::to_string(&Mods::empty()).unwrap(), r#""""#);
        assert_eq!(
            serde_json::to_string(&Mods::all()).unwrap(),
            r#""SHIFT | CTRL | ALT | SUPER""#
        );
        // bincode is not human readable, so the socket carries the bits, not the names.
        assert_eq!(bincode::serialize(&mods).unwrap(), vec![0b11]);
        assert_eq!(bincode::deserialize::<Mods>(&[0b11]).unwrap(), mods);
    }
}
