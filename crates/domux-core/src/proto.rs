//! The attach protocol. Task 9 fills this file; the Model needs `Capabilities` first.

use crate::ids::ClientId;
use domux_term::{CursorShape, KeyEvent, Rgb};
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
pub const PROTOCOL_VERSION: u32 = 1;
/// A frame larger than this is a bug or an attack, never a screen.
pub const MAX_FRAME: u32 = 64 * 1024 * 1024;

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

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("frame of {0} bytes exceeds the {MAX_FRAME} byte limit")]
    FrameTooLarge(u32),
    #[error("could not decode a message: {0}")]
    Decode(String),
}

/// A message as bytes: a 4-byte little-endian length, then bincode.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let body = bincode::serialize(msg).expect("serializable message");
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
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

    // Not `Iterator::next`: the message type is chosen per call, and decoding can fail.
    #[allow(clippy::should_implement_trait)]
    pub fn next<T: serde::de::DeserializeOwned>(&mut self) -> Result<Option<T>, ProtoError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
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
/// starts with a length whose first byte is `{` only for a 2 GB hello, which never happens.
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
        let mut bytes = encode(&hello);
        bytes.extend(encode(&key));
        let mut d = Decoder::default();
        // Feed one byte at a time to prove partial frames are buffered.
        let mut out: Vec<ClientMsg> = Vec::new();
        for b in bytes {
            d.push(&[b]);
            while let Some(m) = d.next::<ClientMsg>().unwrap() {
                out.push(m);
            }
        }
        assert_eq!(out, vec![hello, key]);
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
        d.push(&encode(&msg));
        assert_eq!(d.next::<ServerMsg>().unwrap(), Some(ServerMsg::Frame(diff)));
        assert_eq!(d.next::<ServerMsg>().unwrap(), None);
    }

    #[test]
    fn oversized_frames_are_rejected_before_allocation() {
        let mut d = Decoder::default();
        d.push(&(MAX_FRAME + 1).to_le_bytes());
        assert!(matches!(
            d.next::<ServerMsg>(),
            Err(ProtoError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn the_first_byte_tells_the_protocols_apart() {
        assert!(is_control_api_first_byte(b'{'));
        assert!(!is_control_api_first_byte(encode(&ClientMsg::Detach)[0]));
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
