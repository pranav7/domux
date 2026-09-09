//! Overlays drawn over the workpanel. M1: the keys help. The tab-name prompt is drawn by
//! `tab_row` in the tab's cell. M2 adds NameWorkspace and Confirm; M4 adds Usage.

use crate::render::boxed::{put_within, Boxed};
use crate::render::{theme, RenderInput};
use domux_core::model::Overlay;
use domux_core::text::{display_width, truncate_with_ellipsis};
use domux_term::Size;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

/// The columns an overlay leaves on the screen, and how far down it starts
/// (interface spec 12.13).
pub const OVERLAY_MARGIN: u16 = 24;
pub const OVERLAY_TOP: u16 = 3;

/// The narrowest and widest a list overlay is allowed to be (interface spec 12.13). The
/// floor is above the sidebar's 38 columns, so whatever fits in the sidebar fits here
/// (interface spec 5.1) without a second clamp saying so.
const LIST_MIN_WIDTH: u16 = 60;
const LIST_MAX_WIDTH: u16 = 120;

/// Between two hints in the footer, the same separator the sidebar's hint row uses.
const HINT_SEP: &str = " · ";

/// Draws the overlay a client has open, and the one it was opened over under it
/// (interface spec 12.7).
pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    if let Some(under) = &input.view.overlay_under {
        draw_one(input, under, buf);
    }
    if let Some(top) = &input.view.overlay {
        draw_one(input, top, buf);
    }
}

fn draw_one(input: &RenderInput, overlay: &Overlay, buf: &mut Buffer) {
    match overlay {
        Overlay::Help => draw_help(input, buf),
        Overlay::Switcher => crate::render::switcher::draw(input, buf),
        Overlay::NameWorkspace(id) => crate::render::name_box::draw(input, id, buf),
        Overlay::Confirm(kind) => crate::render::confirm::draw(input, kind, buf),
        // The tab prompt lives in the tab cell; Agents is M3 and Usage is M4.
        Overlay::Prompt(_) | Overlay::Agents | Overlay::Usage => {}
    }
}

/// How wide a list overlay is on a screen of this width: the screen less 24 columns, at
/// least 60 and at most 120, and never wider than the screen less its own two margin
/// columns.
///
/// Answered on its own because the switcher needs the width before it has the rows: the
/// rows truncate to the box's inner width, and the row count then decides the height. The
/// width does not depend on the rows, so the two passes cannot disagree.
///
/// The screen bound is applied last on purpose. Interface spec 5.1's "never narrower than
/// the sidebar" is already carried by the 60-column floor, and clamping up to 38 after the
/// screen bound would put the box outside a screen narrower than 40 columns, where
/// `Boxed::render` indexes past the buffer rather than clipping.
pub fn list_overlay_width(screen: Rect) -> u16 {
    screen
        .width
        .saturating_sub(OVERLAY_MARGIN)
        .clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH)
        .min(screen.width.saturating_sub(2))
}

/// The rectangle a list overlay takes: `list_overlay_width` wide, centred, three rows from
/// the top, its height fitting `lines` up to the screen less six, and always with a row
/// left under it on the screen for the footer (interface spec 12.13).
pub fn list_overlay_area(screen: Rect, lines: u16) -> Rect {
    let width = list_overlay_width(screen);
    let max_height = screen.height.saturating_sub(6).max(3);
    let height = lines
        .saturating_add(2)
        .clamp(3, max_height)
        // The footer's row is the screen's, not the box's, so the box gives it up rather
        // than drawing a footer past the bottom row.
        .min(screen.height.saturating_sub(OVERLAY_TOP + 1));
    Rect::new(
        screen.x + screen.width.saturating_sub(width) / 2,
        screen.y + OVERLAY_TOP.min(screen.height.saturating_sub(height + 1)),
        width,
        height,
    )
}

/// Paints `area` in the overlay's background, so nothing of the screen shows through.
pub fn clear(area: Rect, buf: &mut Buffer) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf[(x, y)].reset();
            buf[(x, y)].set_style(Style::default().bg(theme::BASE));
        }
    }
}

/// The same box `frame` draws, at a rectangle the caller chose. Returns the inner area.
///
/// A list overlay calls `clear` instead and lets its `ListBox` draw the border, so the
/// switcher and the sidebar share one drawing of the Projects box. The name box and the
/// confirmation are not list boxes and take the whole thing.
pub fn frame_at(title: &str, area: Rect, buf: &mut Buffer) -> Rect {
    clear(area, buf);
    Boxed {
        title,
        flag: None,
        focused: true,
    }
    .render(area, buf)
}

/// Dims every cell outside `keep`, so the screen reads as being behind the overlay
/// (interface spec 7.1). Colour is not the only carrier: the overlay's border and its
/// cleared background already separate it.
///
/// Called after the overlay has drawn itself, so `keep` is what decides that the overlay is
/// not dimmed with the screen. Dimming first and relying on the overlay's own clear to undo
/// it would leave `keep` doing nothing that could be seen.
pub fn dim(buf: &mut Buffer, keep: &[Rect]) {
    for y in buf.area.y..buf.area.bottom() {
        for x in buf.area.x..buf.area.right() {
            if keep.iter().any(|r| r.contains((x, y).into())) {
                continue;
            }
            buf[(x, y)].modifier.insert(Modifier::DIM);
        }
    }
}

/// `⏎ open · / filter · ? help · esc close`, rendered from the configured `[keys.list]`
/// bindings (principle 3). While `/` is being typed it becomes `Filter › text▮  esc clear`
/// (interface spec 12.10); a pill replaces both until it clears.
///
/// It lives here rather than in one overlay because it is the overlay frame's row: the
/// switcher draws it and M3's agents overlay draws the same one (roadmap 5.9).
///
/// No input reaches the degenerate-area guard below, so no test pins it and removing it
/// breaks nothing today: the only caller derives the row from `list_overlay_area`, and
/// `a_list_overlay_and_its_footer_row_stay_inside_any_screen` sweeps that function for
/// exactly this promise. It is defence for the rectangles Tasks 15, 16 and 18 will pass.
pub fn footer(input: &RenderInput, hints: &[(&str, &str)], area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 || area.y >= buf.area.bottom() {
        return;
    }
    let base = Style::default().bg(theme::BASE);
    // The row is the footer's: whatever the screen had there is gone, so the overlay is
    // read as one thing with its keys under it rather than as a box over someone's text.
    clear(Rect::new(area.x, area.y, area.width, 1), buf);
    let y = area.y;
    let x = area.x + 1;
    // One cell of padding at each end, the same as a box's title.
    let last_x = area.x + area.width.saturating_sub(2);
    let width = area.width.saturating_sub(2) as usize;
    let key_style = base.fg(theme::BLUE);
    let word_style = base.fg(theme::OVERLAY0);
    let sep_style = base.fg(theme::SURFACE1);
    if let Some(pill) = &input.view.pill {
        let style = base
            .fg(theme::BASE)
            .bg(if pill.ok { theme::GREEN } else { theme::RED })
            .add_modifier(Modifier::BOLD);
        put_within(
            buf,
            x,
            y,
            last_x,
            &truncate_with_ellipsis(&pill.text, width),
            style,
        );
        return;
    }
    if input.view.filtering {
        // `esc clear` is spelled out rather than looked up: no action clears the filter, so
        // there is no binding to read. Task 14 gives Esc that meaning while `/` is open.
        let mut cx = put_within(buf, x, y, last_x, "Filter › ", word_style);
        cx = put_within(buf, cx, y, last_x, &input.view.filter, base.fg(theme::TEXT));
        cx = put_within(
            buf,
            cx,
            y,
            last_x,
            " ",
            base.add_modifier(Modifier::REVERSED),
        );
        put_within(buf, cx, y, last_x, "  esc clear", word_style);
        return;
    }
    // What the start-up prune took away, over the keys: the keys are the same on every frame
    // and the note is on this one only. Under the pill and under the filter, which are both
    // answers to something the reader just did, where a note is about what happened before
    // they arrived. The filter cannot in fact be open with a note showing - typing `/` is a
    // key in a box and clears the notes - so that half of the order is a statement of intent
    // rather than a case any input reaches today.
    if let Some(note) = crate::render::note_line(input.notes) {
        put_within(
            buf,
            x,
            y,
            last_x,
            &truncate_with_ellipsis(&note, width),
            base.fg(theme::TEXT),
        );
        return;
    }
    let mut cx = x;
    for (action, label) in hints {
        // A key the reader has rebound to nothing drops out of the row rather than naming a
        // key that does nothing (principle 3).
        let Some(k) = input.keymap.list_key_for(action) else {
            continue;
        };
        let text = format!(" {label}");
        // The separator belongs to the hint after it, so a hint too wide for what is left
        // takes its separator with it and the row never ends on a dangling ` · `. `cx > x`
        // and not "this is not the first hint": a first hint whose key is unbound draws
        // nothing, and counting instead would open the row with a separator.
        let sep_width = if cx > x { display_width(HINT_SEP) } else { 0 };
        // Whole hints only, and `break` rather than `continue`: `hints` is in the order the
        // reader should see them, so keeping a narrower later one over an earlier one would
        // reorder the row.
        if sep_width + display_width(&k) + display_width(&text) > (last_x + 1 - cx) as usize {
            break;
        }
        if sep_width > 0 {
            cx = put_within(buf, cx, y, last_x, HINT_SEP, sep_style);
        }
        cx = put_within(buf, cx, y, last_x, &k, key_style);
        cx = put_within(buf, cx, y, last_x, &text, word_style);
    }
}

/// The centred box every overlay draws into (roadmap 5.1): `width` by `height` cells,
/// clamped to the workpanel, cleared to `base`, with a focused `Boxed` titled `title`. Returns
/// the inner area. M2's switcher, name-workspace and confirmation overlays call this.
///
/// Centred over the workpanel, not over the whole screen. The top bar carries the tab row and
/// the clock and stays readable under every overlay, and a box centred over the screen puts its
/// own border on the pane box's border row whenever the two heights are close: `┌ ┌ Keys ─┐─┐`
/// on an 80x24 or 40x10 screen, which reads as one broken box rather than two boxes
/// (principle 14).
///
/// Clamped twice: to the wanted size less a margin, and then to the workpanel itself.
/// `Boxed::render` indexes the buffer by row without checking it, so an area one row past the
/// edge panics rather than clips, and a workpanel smaller than the smallest box gets no box at
/// all rather than one drawn outside itself.
pub fn frame(title: &str, width: u16, height: u16, buf: &mut Buffer) -> Rect {
    let area = centred_area(width, height, buf);
    if area.width == 0 || area.height == 0 {
        return area;
    }
    frame_at(title, area, buf)
}

/// Where `frame` puts a `width` by `height` box, without drawing anything. Answered on its
/// own for the confirmation, which asks its question in the border in red (interface spec
/// 7.3): `Boxed` draws a title in the accent or in `overlay1` and in nothing else, so the
/// confirmation frames an empty title and writes the question into the border row itself,
/// which needs the outer rectangle rather than the inner one.
pub fn centred_area(width: u16, height: u16, buf: &Buffer) -> Rect {
    let screen = buf.area;
    // The whole width, sidebar or no sidebar: an overlay is the one thing on the screen the
    // keys go to, so it is centred on the screen and covers the sidebar like anything else.
    let workpanel = crate::render::workpanel_of(
        Size {
            cols: screen.width,
            rows: screen.height,
        },
        false,
    );
    let panel = Rect::new(
        screen.x + workpanel.x,
        screen.y + workpanel.y,
        workpanel.width,
        workpanel.height,
    );
    let width = width
        .min(panel.width.saturating_sub(4))
        .max(20)
        .min(panel.width);
    let height = height
        .min(panel.height.saturating_sub(2))
        .max(3)
        .min(panel.height);
    if width == 0 || height == 0 {
        return Rect::new(panel.x, panel.y, 0, 0);
    }
    Rect::new(
        panel.x + (panel.width - width) / 2,
        panel.y + (panel.height - height) / 2,
        width,
        height,
    )
}

/// `┌ Keys ┐`: the leader, every `[keys.bindings]` line as `C-a |    pane.split right`,
/// every `[keys.global]` line, the passthrough rule, and `esc close`. Rendered from the
/// loaded keymap, so a rebinding shows here (principle 3).
fn draw_help(input: &RenderInput, buf: &mut Buffer) {
    let km = input.keymap;
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("leader {}", km.leader));
    lines.push(String::new());
    // Collapse the run of `tab.select <n>` bindings into one row. Nine near-identical rows
    // push the globals, the passthrough rule and the footer past the bottom of the box on
    // an 80x24 screen, and the row still renders the configured keys (principle 3).
    let mut digits: Vec<String> = km
        .bindings
        .iter()
        .filter(|b| b.action.to_string().starts_with("tab.select "))
        .map(|b| b.key.to_string())
        .collect();
    digits.sort();
    let mut bindings: Vec<(String, String)> = km
        .bindings
        .iter()
        .filter(|b| !b.action.to_string().starts_with("tab.select "))
        .map(|b| (format!("{} {}", km.leader, b.key), b.action.to_string()))
        .collect();
    if let (Some(first), Some(last)) = (digits.first(), digits.last()) {
        let keys = if digits.len() == 1 {
            first.clone()
        } else {
            format!("{first}-{last}")
        };
        bindings.push((format!("{} {}", km.leader, keys), "tab.select <n>".into()));
    }
    bindings.sort_by(|a, b| a.1.cmp(&b.1));
    let mut globals: Vec<(String, String)> = km
        .global
        .iter()
        .map(|b| (b.key.to_string(), b.action.to_string()))
        .collect();
    globals.sort_by(|a, b| a.1.cmp(&b.1));
    for (k, a) in bindings.iter().chain(globals.iter()) {
        lines.push(format!("{k:<10} {a}"));
    }
    if !km.passthrough_commands.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "{} keep {}",
            km.passthrough_commands.join(", "),
            km.passthrough_keys
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lines.push(String::new());
    let inner = frame("Keys", 60, lines.len() as u16 + 3, buf);
    if inner.width < 3 || inner.height == 0 {
        return;
    }
    // The footer owns the last inner row whatever else fits, so the overlay always shows the
    // way out of itself (principle 9). On a screen too short for every binding the list
    // truncates and says so; it never silently swallows `esc close`.
    let body_rows = inner.height.saturating_sub(1) as usize;
    let truncated = lines.len() > body_rows;
    let shown = if truncated {
        body_rows.saturating_sub(1)
    } else {
        lines.len()
    };
    let width = inner.width.saturating_sub(2) as usize;
    let last_x = inner.x + inner.width - 1;
    let text = Style::default().fg(theme::TEXT).bg(theme::BASE);
    for (i, line) in lines.iter().take(shown).enumerate() {
        put_within(
            buf,
            inner.x + 1,
            inner.y + i as u16,
            last_x,
            &truncate_with_ellipsis(line, width),
            text,
        );
    }
    if truncated {
        let more = lines.len() - shown;
        let note = format!("{more} more, see domux.toml");
        let style = Style::default().fg(theme::SUBTEXT0).bg(theme::BASE);
        put_within(
            buf,
            inner.x + 1,
            inner.y + shown as u16,
            last_x,
            &truncate_with_ellipsis(&note, width),
            style,
        );
    }
    let footer = Style::default().fg(theme::BLUE).bg(theme::BASE);
    put_within(
        buf,
        inner.x + 1,
        inner.y + inner.height - 1,
        last_x,
        "esc close",
        footer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::FactRegistry;
    use domux_core::ids::{ClientId, PaneId, TabId, WorkspaceId};
    use domux_core::keymap::Keymap;
    use domux_core::model::{ClientView, Focus, Model, Pill, TextInput};
    use domux_core::proto::Capabilities;
    use std::collections::HashMap;

    fn view() -> ClientView {
        ClientView {
            id: ClientId("c_0001".into()),
            size: Size { cols: 80, rows: 24 },
            caps: Capabilities::default(),
            workspace: WorkspaceId("w_0001".into()),
            tab: TabId("t_0001".into()),
            focus: Focus::Pane(PaneId("p_0001".into())),
            sidebar_open: false,
            sidebar_forced: false,
            overlay: None,
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            filtering: false,
            input: TextInput::new(""),
            overlay_under: None,
            pill: None,
        }
    }

    /// Draws the footer for `view` into `area` of `buf`, leaving whatever else is in `buf`.
    fn footer_into(view: &ClientView, area: Rect, buf: &mut Buffer) {
        hints_into(
            view,
            &[("list.activate", "open"), ("focus.pane", "close")],
            area,
            buf,
        );
    }

    /// The same, for a test that needs hints of its own.
    fn hints_into(view: &ClientView, hints: &[(&str, &str)], area: Rect, buf: &mut Buffer) {
        let model = Model::new(1);
        let facts = FactRegistry::new();
        let panes = HashMap::new();
        let keymap = Keymap::defaults();
        let input = RenderInput {
            model: &model,
            facts: &facts,
            panes: &panes,
            view,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
            notes: &[],
        };
        footer(&input, hints, area, buf);
    }

    fn line_of(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// A buffer with every cell holding `X`, so a draw that clears nothing is visible.
    fn filled(w: u16, h: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        for y in 0..h {
            for x in 0..w {
                buf[(x, y)].set_symbol("X");
            }
        }
        buf
    }

    fn screen(cols: u16, rows: u16) -> Rect {
        Rect::new(0, 0, cols, rows)
    }

    /// The width the interface spec's three cases give, read off the formula rather than
    /// through a frame: the screen less the margin, the 120 cap, and the 60 floor.
    #[test]
    fn a_list_overlay_is_the_screen_less_the_margin_between_60_and_120_columns() {
        assert_eq!(list_overlay_width(screen(80, 24)), 60, "the floor holds");
        assert_eq!(
            list_overlay_width(screen(100, 24)),
            76,
            "the margin decides"
        );
        assert_eq!(list_overlay_width(screen(200, 50)), 120, "the cap holds");
        assert_eq!(
            list_overlay_width(screen(60, 12)),
            58,
            "a screen with no room for the floor keeps its own two columns"
        );
    }

    /// The box and its footer row stay inside the screen at every size, including screens
    /// too small to draw on at all.
    ///
    /// Down to 1x1 rather than down to the 40x10 minimum, because this is a pure function and
    /// the two bounds that hold the invariant on a tiny screen - the top pulled up, and the
    /// height giving up the footer's row - are each redundant above the minimum. Swept only
    /// over usable screens, either one could be deleted and the sweep stayed green; the
    /// screens where each is the one doing the work are the ones `compose` refuses to draw.
    /// `Boxed::render` indexes the buffer without checking, so a rectangle past the edge is a
    /// panic and not a clipped box.
    #[test]
    fn a_list_overlay_and_its_footer_row_stay_inside_any_screen() {
        for cols in [1u16, 2, 3, 20, 39, 40, 41, 60, 80, 119, 120, 200, 400] {
            for rows in [1u16, 2, 3, 4, 5, 9, 10, 11, 12, 24, 25, 50, 200] {
                for lines in [0u16, 1, 3, 5, 20, 100, u16::MAX] {
                    let s = screen(cols, rows);
                    let a = list_overlay_area(s, lines);
                    let what = format!("{cols}x{rows}, {lines} lines: {a:?}");
                    assert!(a.x + a.width <= cols, "past the right edge at {what}");
                    assert!(
                        a.y + a.height < rows,
                        "no row left for the footer at {what}"
                    );
                }
            }
        }
    }

    /// On every screen domux actually draws on, the box is centred, at least as wide as the
    /// sidebar, and tall enough to be a box.
    #[test]
    fn a_list_overlay_is_centred_and_never_narrower_than_the_sidebar() {
        for cols in [crate::render::MIN_COLS, 41, 60, 80, 119, 120, 200, 400] {
            for rows in [crate::render::MIN_ROWS, 11, 12, 24, 25, 50, 200] {
                for lines in [0u16, 1, 3, 5, 20, 100, u16::MAX] {
                    let a = list_overlay_area(screen(cols, rows), lines);
                    let what = format!("{cols}x{rows}, {lines} lines: {a:?}");
                    assert!(
                        a.width >= domux_core::model::SIDEBAR_WIDTH,
                        "narrower than the sidebar at {what}"
                    );
                    assert!(a.height >= 3, "shorter than a box at {what}");
                    let right_margin = cols - (a.x + a.width);
                    assert!(
                        a.x.abs_diff(right_margin) <= 1,
                        "not centred at {what}: {} left, {right_margin} right",
                        a.x
                    );
                }
            }
        }
    }

    /// The screen is a rectangle and not a size, so the box lands inside it wherever it
    /// starts. `compose` builds its buffer at the origin and is the only caller today, so
    /// this is the one fixture that can tell an area measured from the screen from one
    /// measured from 0,0. Tasks 15, 16 and 18 are the next callers.
    #[test]
    fn a_list_overlay_sits_inside_the_screen_it_was_given_wherever_that_starts() {
        let moved = list_overlay_area(Rect::new(7, 2, 80, 24), 3);
        let at_origin = list_overlay_area(screen(80, 24), 3);
        assert_eq!(moved.width, at_origin.width);
        assert_eq!(moved.height, at_origin.height);
        assert_eq!(moved.x, at_origin.x + 7);
        assert_eq!(moved.y, at_origin.y + 2);
    }

    /// The height follows the rows until the screen runs out, and then stops.
    #[test]
    fn a_list_overlay_grows_with_its_rows_up_to_the_screen_less_six() {
        assert_eq!(list_overlay_area(screen(80, 24), 0).height, 3);
        assert_eq!(list_overlay_area(screen(80, 24), 3).height, 5);
        assert_eq!(list_overlay_area(screen(80, 24), 16).height, 18);
        assert_eq!(
            list_overlay_area(screen(80, 24), 40).height,
            18,
            "24 - 6, so six rows of screen stay readable behind it"
        );
        assert_eq!(
            list_overlay_area(screen(80, 24), 0).y,
            OVERLAY_TOP,
            "three rows from the top"
        );
    }

    /// The box lands where it was told inside a bigger buffer, and writes nothing outside
    /// itself. Both halves matter: a box drawn in the right place that overruns its bounds
    /// has the same symptom as one drawn in the wrong place.
    #[test]
    fn frame_at_draws_at_its_rectangle_and_clears_only_that() {
        let mut buf = filled(20, 6);
        let inner = frame_at("T", Rect::new(4, 1, 10, 3), &mut buf);
        assert_eq!(inner, Rect::new(5, 2, 8, 1));
        assert_eq!(line_of(&buf, 0), "X".repeat(20));
        assert_eq!(line_of(&buf, 1), "XXXX┌ T ─────┐XXXXXX");
        assert_eq!(
            line_of(&buf, 2),
            "XXXX│        │XXXXXX",
            "the inside is cleared, not left holding what was under it"
        );
        assert_eq!(line_of(&buf, 3), "XXXX└────────┘XXXXXX");
        assert_eq!(line_of(&buf, 4), "X".repeat(20));
        assert_eq!(buf[(4, 2)].bg, theme::BASE, "and it is cleared to base");
    }

    /// Everything outside `keep` is dimmed and nothing inside it is, and the text is left
    /// where it was: dimming is an attribute over the screen, not a repaint of it.
    #[test]
    fn dim_marks_everything_outside_what_it_was_told_to_keep() {
        let mut buf = filled(6, 3);
        dim(&mut buf, &[Rect::new(2, 1, 2, 1)]);
        for (x, y, want) in [
            (0u16, 0u16, true),
            (2, 0, true),
            (1, 1, true),
            (2, 1, false),
            (3, 1, false),
            (4, 1, true),
            // The screen's last column, which is the one a sweep stopping one short of the
            // right edge would leave bright under the overlay.
            (5, 1, true),
            (2, 2, true),
        ] {
            assert_eq!(
                buf[(x, y)].modifier.contains(Modifier::DIM),
                want,
                "at {x},{y}"
            );
            assert_eq!(buf[(x, y)].symbol(), "X", "at {x},{y}");
        }
    }

    /// The row is the footer's: what was there is gone, and nothing past its own columns is
    /// touched.
    #[test]
    fn the_footer_clears_its_row_and_draws_inside_its_own_columns() {
        let mut buf = filled(30, 3);
        footer_into(&view(), Rect::new(4, 1, 20, 1), &mut buf);
        assert_eq!(line_of(&buf, 0), "X".repeat(30));
        assert_eq!(line_of(&buf, 1), "XXXX ⏎ open · esc close XXXXXX");
        assert_eq!(line_of(&buf, 2), "X".repeat(30));
        // The separator is the row's quietest colour, under both the key and the word, so a
        // reader's eye lands on the keys. Nothing else here reads a style off the footer.
        assert_eq!(buf[(12, 1)].symbol(), "·");
        assert_eq!(buf[(12, 1)].fg, theme::SURFACE1);
        assert_eq!(buf[(11, 1)].fg, theme::SURFACE1, "and its spaces with it");
    }

    /// A hint the row has no room for ends the row: the hints are in the order the reader
    /// should see them, so a later one that would fit is dropped with it rather than moving
    /// up into the gap.
    ///
    /// The middle hint is the widest, which is what separates the two. `HINTS` in the
    /// switcher happens to end with its widest, `esc close`, so its own footer draws the
    /// same row either way; `footer` is public and takes whatever hints it is given, and
    /// `[keys.list]` decides how wide each one is.
    ///
    /// 15 cells of room: `⏎ open` is 6, ` · esc close` is 12 and does not fit, ` · ? help`
    /// is 9 and would.
    #[test]
    fn the_footer_stops_at_the_first_hint_that_does_not_fit_rather_than_keeping_a_later_one() {
        let mut buf = filled(20, 1);
        hints_into(
            &view(),
            &[
                ("list.activate", "open"),
                ("focus.pane", "close"),
                ("help", "help"),
            ],
            Rect::new(0, 0, 17, 1),
            &mut buf,
        );
        assert_eq!(line_of(&buf, 0), " ⏎ open          XXX");
    }

    /// A hint that does not fit the row is dropped whole, with its separator, rather than
    /// cut in half or left as a dangling ` · ` at the end of the row.
    #[test]
    fn the_footer_drops_a_hint_that_does_not_fit_instead_of_cutting_it() {
        let mut buf = filled(30, 1);
        footer_into(&view(), Rect::new(4, 0, 10, 1), &mut buf);
        assert_eq!(line_of(&buf, 0), "XXXX ⏎ open   XXXXXXXXXXXXXXXX");
    }

    /// The hints keep a cell of padding at each end of the row, the same as a box's title.
    ///
    /// `⏎ open · esc close` is 18 cells, so a 20-column footer is the narrowest that holds
    /// it and a 19-column one drops the last hint. One cell either way here, and both cases
    /// are needed: a footer measured to the row's last column instead of one short of it
    /// draws both hints at 19 and reads as text pressed against the border.
    #[test]
    fn the_footer_keeps_a_cell_of_padding_at_each_end() {
        let mut wide = filled(20, 1);
        footer_into(&view(), Rect::new(0, 0, 20, 1), &mut wide);
        assert_eq!(line_of(&wide, 0), " ⏎ open · esc close ");
        let mut narrow = filled(19, 1);
        footer_into(&view(), Rect::new(0, 0, 19, 1), &mut narrow);
        assert_eq!(line_of(&narrow, 0), " ⏎ open            ");
    }

    /// While `/` is being typed the footer is the filter and the way to clear it
    /// (interface spec 12.10), and the caret is drawn at the end of the text.
    #[test]
    fn the_footer_shows_the_filter_while_one_is_being_typed() {
        let mut v = view();
        v.filtering = true;
        v.filter = "auth".into();
        let mut buf = filled(40, 1);
        footer_into(&v, Rect::new(0, 0, 40, 1), &mut buf);
        assert_eq!(line_of(&buf, 0), " Filter › auth   esc clear              ");
        assert!(
            buf[(14, 0)].modifier.contains(Modifier::REVERSED),
            "the caret is the cell after the text"
        );
    }

    /// A result displaces the keys until it clears (interface spec 7.3 and 12.12), and it
    /// is cut to the row rather than running past it.
    #[test]
    fn a_pill_takes_the_footer_while_it_is_showing() {
        let mut v = view();
        v.pill = Some(Pill {
            text: "w".repeat(40),
            ok: false,
            at: String::new(),
        });
        let mut buf = filled(20, 1);
        footer_into(&v, Rect::new(0, 0, 20, 1), &mut buf);
        assert_eq!(line_of(&buf, 0), format!(" {}… ", "w".repeat(17)));
        assert_eq!(buf[(1, 0)].bg, theme::RED, "red because it was refused");
        v.pill.as_mut().unwrap().ok = true;
        let mut buf = filled(20, 1);
        footer_into(&v, Rect::new(0, 0, 20, 1), &mut buf);
        assert_eq!(buf[(1, 0)].bg, theme::GREEN);
    }
}
