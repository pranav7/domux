//! libghostty-vt behind the `Emulator` trait.
//!
//! Ownership: `GhosttyEmulator` owns the terminal, a render state, a row iterator, a row
//! cells cursor, a key encoder, and a boxed `Callbacks` whose address is registered with the
//! terminal as user data. The terminal is freed before the box drops. The handles are not
//! thread-safe; the type is `Send` because it is only ever used from the task that owns it,
//! and never `Sync`.

pub mod ffi;

use crate::emulator::{Emulator, EmulatorConfig};
use crate::key::{Key, KeyAction, KeyEvent, Mods};
use crate::types::{Attrs, Cell, Color, Cursor, CursorShape, Grid, Rgb, Size};
use std::ffi::c_void;
use std::ptr;

/// Packs a mode number the way `ghostty_mode_new` does. That function is a static inline in
/// the header, so bindgen does not generate it. Bit 15 marks an ANSI mode; DEC private modes
/// (the ones domux queries) leave it clear.
const fn mode(value: u16, ansi: bool) -> ffi::GhosttyMode {
    (value & 0x7FFF) | ((ansi as u16) << 15)
}

const MODE_BRACKETED_PASTE: ffi::GhosttyMode = mode(2004, false);

/// State the C callbacks write into. Lives in a Box so its address is stable.
struct Callbacks {
    responses: Vec<u8>,
    bell: bool,
}

/// A libghostty handle paired with its destructor, so dropping it frees it. A failure part
/// way through `GhosttyEmulator::new` then releases the handles already built, and adding a
/// handle costs one line instead of another rung on a cleanup ladder.
struct Handle<T: Copy> {
    raw: T,
    free: unsafe extern "C" fn(T),
}

impl<T: Copy> Drop for Handle<T> {
    fn drop(&mut self) {
        unsafe { (self.free)(self.raw) };
    }
}

/// Builds one of the handles whose constructor takes an allocator and an out pointer. `name`
/// is the C function, named back in the error when it fails.
fn new_handle<T: Copy>(
    name: &str,
    new: unsafe extern "C" fn(*const ffi::GhosttyAllocator, *mut T) -> ffi::GhosttyResult,
    free: unsafe extern "C" fn(T),
) -> Result<Handle<T>, String> {
    let mut raw = std::mem::MaybeUninit::<T>::uninit();
    if unsafe { new(ptr::null(), raw.as_mut_ptr()) } != ffi::GhosttyResult_GHOSTTY_SUCCESS {
        return Err(format!("{name} failed"));
    }
    Ok(Handle {
        raw: unsafe { raw.assume_init() },
        free,
    })
}

/// Fields drop in declaration order, so listing the handles in reverse order of creation
/// frees them in the order libghostty expects, and `callbacks` outlives the terminal that
/// holds its address.
pub struct GhosttyEmulator {
    key_encoder: Handle<ffi::GhosttyKeyEncoder>,
    row_cells: Handle<ffi::GhosttyRenderStateRowCells>,
    row_iterator: Handle<ffi::GhosttyRenderStateRowIterator>,
    render_state: Handle<ffi::GhosttyRenderState>,
    terminal: Handle<ffi::GhosttyTerminal>,
    callbacks: Box<Callbacks>,
    size: Size,
}

// Safety: every handle is used only from the task that owns the emulator. The type is never
// shared between threads, so it is `Send` but deliberately not `Sync`.
unsafe impl Send for GhosttyEmulator {}

/// Called by libghostty-vt when the terminal owes the inner program bytes (DSR replies, mode
/// reports, in-band size reports). The data is only valid for the call, so it is copied.
unsafe extern "C" fn write_pty(
    _terminal: ffi::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    let cb = unsafe { &mut *(userdata as *mut Callbacks) };
    cb.responses
        .extend_from_slice(unsafe { std::slice::from_raw_parts(data, len) });
}

unsafe extern "C" fn bell(_terminal: ffi::GhosttyTerminal, userdata: *mut c_void) {
    let cb = unsafe { &mut *(userdata as *mut Callbacks) };
    cb.bell = true;
}

/// Answers device attributes queries. Without this callback libghostty-vt drops them, and
/// programs that wait for a DA1 reply (neovim, Claude Code) stall at start. The values match
/// what Ghostty itself reports: a VT220 with 132 columns, printer, selective erase, and
/// technical characters.
unsafe extern "C" fn device_attributes(
    _terminal: ffi::GhosttyTerminal,
    _userdata: *mut c_void,
    out_attrs: *mut ffi::GhosttyDeviceAttributes,
) -> bool {
    let attrs = unsafe { &mut *out_attrs };
    attrs.primary.conformance_level = 62; // VT220
    let features = [1u16, 2, 6, 22]; // 132 columns, printer, selective erase, ANSI color
    attrs.primary.features[..features.len()].copy_from_slice(&features);
    attrs.primary.num_features = features.len();
    attrs.secondary.device_type = 1;
    attrs.secondary.firmware_version = 10;
    attrs.secondary.rom_cartridge = 0;
    attrs.tertiary.unit_id = 0;
    true
}

/// The C setters take a bare `void*`, so the casts above lose the signature check. These
/// bindings restore it: each fails to compile if a vendor bump changes the callback's shape.
const _: ffi::GhosttyTerminalWritePtyFn = Some(write_pty);
const _: ffi::GhosttyTerminalBellFn = Some(bell);
const _: ffi::GhosttyTerminalDeviceAttributesFn = Some(device_attributes);

fn rgb(c: Rgb) -> ffi::GhosttyColorRgb {
    ffi::GhosttyColorRgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

impl GhosttyEmulator {
    pub fn new(config: EmulatorConfig) -> Result<Self, String> {
        let mut callbacks = Box::new(Callbacks {
            responses: Vec::new(),
            bell: false,
        });
        let userdata = &mut *callbacks as *mut Callbacks as *mut c_void;

        let mut raw_terminal: ffi::GhosttyTerminal = ptr::null_mut();
        if unsafe {
            ffi::ghostty_terminal_new(
                ptr::null(),
                &mut raw_terminal,
                config.size.cols,
                config.size.rows,
            )
        } != ffi::GhosttyResult_GHOSTTY_SUCCESS
        {
            return Err("ghostty_terminal_new failed".into());
        }
        let terminal = Handle {
            raw: raw_terminal,
            free: ffi::ghostty_terminal_free,
        };

        let scrollback = config.scrollback_lines;
        let fg = rgb(config.default_fg);
        let bg = rgb(config.default_bg);
        // Userdata first: every callback below receives it. The default colors are what OSC
        // 10 and OSC 11 queries answer with.
        let options: [(ffi::GhosttyTerminalOption, *const c_void); 7] = [
            (
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_USERDATA,
                userdata,
            ),
            (
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                write_pty as *const c_void,
            ),
            (
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_BELL,
                bell as *const c_void,
            ),
            (
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_DEVICE_ATTRIBUTES,
                device_attributes as *const c_void,
            ),
            (
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                &scrollback as *const usize as *const c_void,
            ),
            (
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND,
                &fg as *const _ as *const c_void,
            ),
            (
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND,
                &bg as *const _ as *const c_void,
            ),
        ];
        for (option, value) in options {
            unsafe { ffi::ghostty_terminal_set(terminal.raw, option, value) };
        }

        // Each `?` below drops the handles already built.
        Ok(GhosttyEmulator {
            render_state: new_handle(
                "ghostty_render_state_new",
                ffi::ghostty_render_state_new,
                ffi::ghostty_render_state_free,
            )?,
            row_iterator: new_handle(
                "ghostty_render_state_row_iterator_new",
                ffi::ghostty_render_state_row_iterator_new,
                ffi::ghostty_render_state_row_iterator_free,
            )?,
            row_cells: new_handle(
                "ghostty_render_state_row_cells_new",
                ffi::ghostty_render_state_row_cells_new,
                ffi::ghostty_render_state_row_cells_free,
            )?,
            key_encoder: new_handle(
                "ghostty_key_encoder_new",
                ffi::ghostty_key_encoder_new,
                ffi::ghostty_key_encoder_free,
            )?,
            terminal,
            callbacks,
            size: config.size,
        })
    }

    /// Reads a DEC private or ANSI mode from the terminal.
    fn mode_enabled(&self, m: ffi::GhosttyMode) -> bool {
        let mut config = ffi::GhosttyTerminalModeConfig {
            mode: m,
            value: false,
        };
        let rc = unsafe {
            ffi::ghostty_terminal_get(
                self.terminal.raw,
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MODE,
                &mut config as *mut _ as *mut c_void,
            )
        };
        rc == ffi::GhosttyResult_GHOSTTY_SUCCESS && config.value
    }

    /// Refreshes the render state from the terminal. `cursor` and `snapshot_grid` both read
    /// it, and `cursor` takes `&self`, so the update happens through a raw handle copy.
    fn update_render_state(&self) {
        unsafe { ffi::ghostty_render_state_update(self.render_state.raw, self.terminal.raw) };
    }
}

impl Emulator for GhosttyEmulator {
    fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        unsafe { ffi::ghostty_terminal_vt_write(self.terminal.raw, bytes.as_ptr(), bytes.len()) };
    }

    fn take_responses(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.callbacks.responses);
    }

    fn resize(&mut self, size: Size) {
        unsafe { ffi::ghostty_terminal_resize(self.terminal.raw, size.cols, size.rows, 0, 0) };
        self.size = size;
    }

    fn size(&self) -> Size {
        self.size
    }

    fn snapshot_grid(&mut self, out: &mut Grid) {
        out.resize(self.size);
        out.clear();
        self.update_render_state();
        unsafe {
            let mut colors = ffi::GhosttyRenderStateColors {
                size: std::mem::size_of::<ffi::GhosttyRenderStateColors>(),
                ..std::mem::zeroed()
            };
            ffi::ghostty_render_state_get(
                self.render_state.raw,
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_COLORS,
                &mut colors as *mut _ as *mut c_void,
            );

            // Bind the reusable iterator to this render state, then walk every row.
            if ffi::ghostty_render_state_get(
                self.render_state.raw,
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                &mut self.row_iterator.raw as *mut _ as *mut c_void,
            ) != ffi::GhosttyResult_GHOSTTY_SUCCESS
            {
                return;
            }
            let mut r: u16 = 0;
            while r < self.size.rows
                && ffi::ghostty_render_state_row_iterator_next(self.row_iterator.raw)
            {
                if ffi::ghostty_render_state_row_get(
                    self.row_iterator.raw,
                    ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    &mut self.row_cells.raw as *mut _ as *mut c_void,
                ) == ffi::GhosttyResult_GHOSTTY_SUCCESS
                {
                    let mut c: u16 = 0;
                    while c < self.size.cols
                        && ffi::ghostty_render_state_row_cells_next(self.row_cells.raw)
                    {
                        fill_cell(self.row_cells.raw, out.cell_mut(r, c));
                        c += 1;
                    }
                }
                r += 1;
            }
        }
    }

    fn cursor(&self) -> Cursor {
        self.update_render_state();
        unsafe {
            let mut c = ffi::GhosttyRenderStateCursor {
                size: std::mem::size_of::<ffi::GhosttyRenderStateCursor>(),
                ..std::mem::zeroed()
            };
            if ffi::ghostty_render_state_get(
                self.render_state.raw,
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR,
                &mut c as *mut _ as *mut c_void,
            ) != ffi::GhosttyResult_GHOSTTY_SUCCESS
            {
                return Cursor::default();
            }
            let shape = match c.visual_style {
                ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE => {
                    CursorShape::Underline
                }
                ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR => {
                    CursorShape::Bar
                }
                _ => CursorShape::Block,
            };
            let (row, col) = if c.viewport_has_value {
                (c.viewport_y, c.viewport_x)
            } else {
                (0, 0)
            };
            Cursor {
                row,
                col,
                visible: c.visible,
                shape,
                blink: c.blinking,
            }
        }
    }

    fn encode_key(&mut self, key: &KeyEvent, out: &mut Vec<u8>) {
        unsafe {
            ffi::ghostty_key_encoder_setopt_from_terminal(self.key_encoder.raw, self.terminal.raw);
            // `setopt_from_terminal` leaves this false because the terminal cannot know it.
            // domux receives key events the client already decoded, so ALT means alt and
            // must produce the escape prefix on every platform.
            let option_as_alt = ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_TRUE;
            ffi::ghostty_key_encoder_setopt(
                self.key_encoder.raw,
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT,
                &option_as_alt as *const _ as *const c_void,
            );
            let mut event: ffi::GhosttyKeyEvent = ptr::null_mut();
            if ffi::ghostty_key_event_new(ptr::null(), &mut event)
                != ffi::GhosttyResult_GHOSTTY_SUCCESS
            {
                return;
            }
            ffi::ghostty_key_event_set_action(
                event,
                match key.action {
                    KeyAction::Press => ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_PRESS,
                    KeyAction::Repeat => ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_REPEAT,
                    KeyAction::Release => ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_RELEASE,
                },
            );
            ffi::ghostty_key_event_set_mods(event, mods_to_ghostty(key.mods));
            let (code, text) = key_to_ghostty(key.key);
            ffi::ghostty_key_event_set_key(event, code);
            if !text.is_empty() {
                ffi::ghostty_key_event_set_utf8(
                    event,
                    text.as_ptr() as *const std::os::raw::c_char,
                    text.len(),
                );
            }
            if let Key::Char(ch) = key.key {
                ffi::ghostty_key_event_set_unshifted_codepoint(
                    event,
                    ch.to_lowercase().next().unwrap_or(ch) as u32,
                );
            }
            // Most encodings are a handful of bytes. On GHOSTTY_OUT_OF_SPACE the encoder
            // reports the size it needs in `written`, so retry once at that size rather than
            // dropping the key.
            let mut buf = [0u8; 128];
            let mut written: usize = 0;
            let rc = ffi::ghostty_key_encoder_encode(
                self.key_encoder.raw,
                event,
                buf.as_mut_ptr() as *mut std::os::raw::c_char,
                buf.len(),
                &mut written,
            );
            match rc {
                ffi::GhosttyResult_GHOSTTY_SUCCESS => out.extend_from_slice(&buf[..written]),
                ffi::GhosttyResult_GHOSTTY_OUT_OF_SPACE => {
                    let mut big = vec![0u8; written];
                    let rc = ffi::ghostty_key_encoder_encode(
                        self.key_encoder.raw,
                        event,
                        big.as_mut_ptr() as *mut std::os::raw::c_char,
                        big.len(),
                        &mut written,
                    );
                    if rc == ffi::GhosttyResult_GHOSTTY_SUCCESS {
                        out.extend_from_slice(&big[..written]);
                    }
                }
                _ => {}
            }
            ffi::ghostty_key_event_free(event);
        }
    }

    fn encode_paste(&self, text: &str, out: &mut Vec<u8>) {
        crate::emulator::wrap_paste(text, self.mode_enabled(MODE_BRACKETED_PASTE), out);
    }
}

/// Copies the cell the row cells iterator is positioned on into a domux `Cell`.
///
/// Safety: `cells` must be a live iterator positioned on a cell.
unsafe fn fill_cell(cells: ffi::GhosttyRenderStateRowCells, out: &mut Cell) {
    // The wide state lives on the raw cell, not the render-state view.
    let mut raw: ffi::GhosttyCell = 0;
    let mut wide = ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_NARROW;
    if unsafe {
        ffi::ghostty_render_state_row_cells_get(
            cells,
            ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
            &mut raw as *mut _ as *mut c_void,
        )
    } == ffi::GhosttyResult_GHOSTTY_SUCCESS
    {
        unsafe {
            ffi::ghostty_cell_get(
                raw,
                ffi::GhosttyCellData_GHOSTTY_CELL_DATA_WIDE,
                &mut wide as *mut _ as *mut c_void,
            )
        };
    }
    out.width = match wide {
        ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_WIDE => 2,
        ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_SPACER_TAIL
        | ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_SPACER_HEAD => 0,
        _ => 1,
    };

    out.text.clear();
    if out.width > 0 {
        let mut len: u32 = 0;
        unsafe {
            ffi::ghostty_render_state_row_cells_get(
                cells,
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                &mut len as *mut _ as *mut c_void,
            )
        };
        if len > 0 {
            // GRAPHEMES_BUF writes exactly `len` codepoints, so the buffer must hold all of
            // them. A fixed-size array would be overrun by a cluster with many combining
            // marks, so anything longer than the common case goes on the heap.
            let len = len as usize;
            let mut stack = [0u32; 16];
            let mut spill = Vec::new();
            let cps: &mut [u32] = if len <= stack.len() {
                &mut stack[..len]
            } else {
                spill.resize(len, 0u32);
                &mut spill
            };
            if unsafe {
                ffi::ghostty_render_state_row_cells_get(
                    cells,
                    ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                    cps.as_mut_ptr() as *mut c_void,
                )
            } == ffi::GhosttyResult_GHOSTTY_SUCCESS
            {
                for &cp in cps.iter() {
                    if let Some(ch) = char::from_u32(cp) {
                        out.text.push(ch);
                    }
                }
            }
        }
        if out.text.is_empty() {
            out.text.push(' ');
        }
    }

    let mut style = ffi::GhosttyStyle {
        size: std::mem::size_of::<ffi::GhosttyStyle>(),
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe {
        ffi::ghostty_render_state_row_cells_get(
            cells,
            ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
            &mut style as *mut _ as *mut c_void,
        )
    } == ffi::GhosttyResult_GHOSTTY_SUCCESS
    {
        out.fg = color_from(style.fg_color);
        out.bg = color_from(style.bg_color);
        out.underline_color = match color_from(style.underline_color) {
            Color::Default => None,
            other => Some(other),
        };
        out.attrs = attrs_from(&style);
    } else {
        out.fg = Color::Default;
        out.bg = Color::Default;
        out.underline_color = None;
        out.attrs = Attrs::empty();
    }
}

fn color_from(c: ffi::GhosttyStyleColor) -> Color {
    unsafe {
        match c.tag {
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE => {
                Color::Indexed(c.value.palette)
            }
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_RGB => {
                let v = c.value.rgb;
                Color::Rgb(Rgb {
                    r: v.r,
                    g: v.g,
                    b: v.b,
                })
            }
            _ => Color::Default,
        }
    }
}

fn attrs_from(style: &ffi::GhosttyStyle) -> Attrs {
    let mut a = Attrs::empty();
    if style.bold {
        a |= Attrs::BOLD;
    }
    if style.faint {
        a |= Attrs::DIM;
    }
    if style.italic {
        a |= Attrs::ITALIC;
    }
    if style.blink {
        a |= Attrs::BLINK;
    }
    if style.inverse {
        a |= Attrs::INVERSE;
    }
    if style.invisible {
        a |= Attrs::INVISIBLE;
    }
    if style.strikethrough {
        a |= Attrs::STRIKETHROUGH;
    }
    if style.overline {
        a |= Attrs::OVERLINE;
    }
    a |= match style.underline {
        ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_SINGLE => Attrs::UNDERLINE,
        ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOUBLE => Attrs::DOUBLE_UNDERLINE,
        ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_CURLY => Attrs::CURLY_UNDERLINE,
        ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOTTED => Attrs::DOTTED_UNDERLINE,
        ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DASHED => Attrs::DASHED_UNDERLINE,
        _ => Attrs::empty(),
    };
    a
}

fn mods_to_ghostty(m: Mods) -> ffi::GhosttyMods {
    // The GHOSTTY_MODS_* constants come through bindgen as u32; the field is u16.
    let mut out: ffi::GhosttyMods = 0;
    if m.contains(Mods::SHIFT) {
        out |= ffi::GHOSTTY_MODS_SHIFT as ffi::GhosttyMods;
    }
    if m.contains(Mods::CTRL) {
        out |= ffi::GHOSTTY_MODS_CTRL as ffi::GhosttyMods;
    }
    if m.contains(Mods::ALT) {
        out |= ffi::GHOSTTY_MODS_ALT as ffi::GhosttyMods;
    }
    if m.contains(Mods::SUPER) {
        out |= ffi::GHOSTTY_MODS_SUPER as ffi::GhosttyMods;
    }
    out
}

/// Maps a logical key to Ghostty's physical key code plus the text it produces. Letters,
/// digits, and ASCII punctuation map by their unshifted US-layout key; other characters have
/// no physical key and are sent as text only.
fn key_to_ghostty(key: Key) -> (ffi::GhosttyKey, String) {
    use ffi::*;
    match key {
        Key::Char(c) => {
            let code = match c.to_ascii_lowercase() {
                'a' => GhosttyKey_GHOSTTY_KEY_A,
                'b' => GhosttyKey_GHOSTTY_KEY_B,
                'c' => GhosttyKey_GHOSTTY_KEY_C,
                'd' => GhosttyKey_GHOSTTY_KEY_D,
                'e' => GhosttyKey_GHOSTTY_KEY_E,
                'f' => GhosttyKey_GHOSTTY_KEY_F,
                'g' => GhosttyKey_GHOSTTY_KEY_G,
                'h' => GhosttyKey_GHOSTTY_KEY_H,
                'i' => GhosttyKey_GHOSTTY_KEY_I,
                'j' => GhosttyKey_GHOSTTY_KEY_J,
                'k' => GhosttyKey_GHOSTTY_KEY_K,
                'l' => GhosttyKey_GHOSTTY_KEY_L,
                'm' => GhosttyKey_GHOSTTY_KEY_M,
                'n' => GhosttyKey_GHOSTTY_KEY_N,
                'o' => GhosttyKey_GHOSTTY_KEY_O,
                'p' => GhosttyKey_GHOSTTY_KEY_P,
                'q' => GhosttyKey_GHOSTTY_KEY_Q,
                'r' => GhosttyKey_GHOSTTY_KEY_R,
                's' => GhosttyKey_GHOSTTY_KEY_S,
                't' => GhosttyKey_GHOSTTY_KEY_T,
                'u' => GhosttyKey_GHOSTTY_KEY_U,
                'v' => GhosttyKey_GHOSTTY_KEY_V,
                'w' => GhosttyKey_GHOSTTY_KEY_W,
                'x' => GhosttyKey_GHOSTTY_KEY_X,
                'y' => GhosttyKey_GHOSTTY_KEY_Y,
                'z' => GhosttyKey_GHOSTTY_KEY_Z,
                '0' | ')' => GhosttyKey_GHOSTTY_KEY_DIGIT_0,
                '1' | '!' => GhosttyKey_GHOSTTY_KEY_DIGIT_1,
                '2' | '@' => GhosttyKey_GHOSTTY_KEY_DIGIT_2,
                '3' | '#' => GhosttyKey_GHOSTTY_KEY_DIGIT_3,
                '4' | '$' => GhosttyKey_GHOSTTY_KEY_DIGIT_4,
                '5' | '%' => GhosttyKey_GHOSTTY_KEY_DIGIT_5,
                '6' | '^' => GhosttyKey_GHOSTTY_KEY_DIGIT_6,
                '7' | '&' => GhosttyKey_GHOSTTY_KEY_DIGIT_7,
                '8' | '*' => GhosttyKey_GHOSTTY_KEY_DIGIT_8,
                '9' | '(' => GhosttyKey_GHOSTTY_KEY_DIGIT_9,
                ' ' => GhosttyKey_GHOSTTY_KEY_SPACE,
                '-' | '_' => GhosttyKey_GHOSTTY_KEY_MINUS,
                '=' | '+' => GhosttyKey_GHOSTTY_KEY_EQUAL,
                '[' | '{' => GhosttyKey_GHOSTTY_KEY_BRACKET_LEFT,
                ']' | '}' => GhosttyKey_GHOSTTY_KEY_BRACKET_RIGHT,
                '\\' | '|' => GhosttyKey_GHOSTTY_KEY_BACKSLASH,
                ';' | ':' => GhosttyKey_GHOSTTY_KEY_SEMICOLON,
                '\'' | '"' => GhosttyKey_GHOSTTY_KEY_QUOTE,
                ',' | '<' => GhosttyKey_GHOSTTY_KEY_COMMA,
                '.' | '>' => GhosttyKey_GHOSTTY_KEY_PERIOD,
                '/' | '?' => GhosttyKey_GHOSTTY_KEY_SLASH,
                '`' | '~' => GhosttyKey_GHOSTTY_KEY_BACKQUOTE,
                _ => GhosttyKey_GHOSTTY_KEY_UNIDENTIFIED,
            };
            (code, c.to_string())
        }
        // Enter, tab, backspace, and escape carry no text: the encoder skips its PC-style
        // function key table whenever an event has utf8, and produces the control byte
        // itself. Ghostty's own encoder tests pass an empty utf8 for these keys.
        Key::Enter => (GhosttyKey_GHOSTTY_KEY_ENTER, String::new()),
        Key::Tab => (GhosttyKey_GHOSTTY_KEY_TAB, String::new()),
        Key::Backspace => (GhosttyKey_GHOSTTY_KEY_BACKSPACE, String::new()),
        Key::Escape => (GhosttyKey_GHOSTTY_KEY_ESCAPE, String::new()),
        Key::Up => (GhosttyKey_GHOSTTY_KEY_ARROW_UP, String::new()),
        Key::Down => (GhosttyKey_GHOSTTY_KEY_ARROW_DOWN, String::new()),
        Key::Left => (GhosttyKey_GHOSTTY_KEY_ARROW_LEFT, String::new()),
        Key::Right => (GhosttyKey_GHOSTTY_KEY_ARROW_RIGHT, String::new()),
        Key::Home => (GhosttyKey_GHOSTTY_KEY_HOME, String::new()),
        Key::End => (GhosttyKey_GHOSTTY_KEY_END, String::new()),
        Key::PageUp => (GhosttyKey_GHOSTTY_KEY_PAGE_UP, String::new()),
        Key::PageDown => (GhosttyKey_GHOSTTY_KEY_PAGE_DOWN, String::new()),
        Key::Insert => (GhosttyKey_GHOSTTY_KEY_INSERT, String::new()),
        Key::Delete => (GhosttyKey_GHOSTTY_KEY_DELETE, String::new()),
        Key::F(n) => (
            match n {
                1 => GhosttyKey_GHOSTTY_KEY_F1,
                2 => GhosttyKey_GHOSTTY_KEY_F2,
                3 => GhosttyKey_GHOSTTY_KEY_F3,
                4 => GhosttyKey_GHOSTTY_KEY_F4,
                5 => GhosttyKey_GHOSTTY_KEY_F5,
                6 => GhosttyKey_GHOSTTY_KEY_F6,
                7 => GhosttyKey_GHOSTTY_KEY_F7,
                8 => GhosttyKey_GHOSTTY_KEY_F8,
                9 => GhosttyKey_GHOSTTY_KEY_F9,
                10 => GhosttyKey_GHOSTTY_KEY_F10,
                11 => GhosttyKey_GHOSTTY_KEY_F11,
                _ => GhosttyKey_GHOSTTY_KEY_F12,
            },
            String::new(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::ffi;

    #[test]
    fn library_links_and_reports_a_codepoint_width() {
        // One call through the static library proves the Zig build and link flags.
        let w = unsafe { ffi::ghostty_unicode_codepoint_width('漢' as u32) };
        assert_eq!(w, 2);
    }
}
