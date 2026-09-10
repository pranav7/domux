//! Where a pointer event goes (decision 0014).
//!
//! The wheel over a pane belongs to that pane's program when the program asked for the mouse,
//! and to copy mode otherwise. The buttons are always domux's: a press focuses what it lands
//! on, a drag selects, a release copies.
//!
//! What a cell belongs to is `render::hit_at`'s answer, asked through the same input the frame
//! is drawn from, so nothing here measures the screen a second time.

use crate::copy_mode::{self, CopyMode};
use crate::core::Core;
use crate::render::Hit;
use domux_core::api::{
    ClientParams, Method, PaneTargetParams, TabCreateParams, TabSelectParams, WorkspaceFocusParams,
};
use domux_core::ids::{ClientId, PaneId};
use domux_core::model::Focus;
use domux_term::{Emulator, Mode, MouseAction, MouseButton, MouseEvent, ScrollbackPos};

/// Ghostty's own default word boundaries, so a double click in a pane selects what a double
/// click in the outer terminal selects. Whitespace is a boundary wherever it appears;
/// these are the rest.
const WORD_BOUNDARIES: &[char] = &[
    '\0', '\'', '"', '│', '`', '|', ':', ';', ',', '(', ')', '[', ']', '{', '}', '<', '>', '$',
];

/// A pointer event that landed in a pane's box: the pane, and the cell of its own grid under
/// the pointer.
struct PaneHit {
    pane: PaneId,
    row: u16,
    col: u16,
}

/// The pane under a screen cell, or `None` when the cell belongs to the chrome or to nothing.
fn pane_hit(core: &Core, client: &ClientId, column: u16, row: u16) -> Option<PaneHit> {
    match core.hit_at(client, column, row) {
        Some(Hit::Pane { pane, row, col }) => Some(PaneHit { pane, row, col }),
        _ => None,
    }
}

/// A wheel gesture. The program is asked first: a program that requested mouse tracking scrolls
/// its own view, and copy mode never opens over it.
///
/// One report for one gesture, whatever `lines` says. The line count is copy mode's step, which
/// exists because the mouse protocol carries no magnitude; the protocol the program reads has
/// no magnitude either, so passing the step on as several reports would invent one.
pub fn wheel(core: &mut Core, client: &ClientId, column: u16, row: u16, lines: i16) {
    if lines == 0 {
        return;
    }
    let Some(hit) = pane_hit(core, client, column, row) else {
        return;
    };
    let button = if lines > 0 {
        MouseButton::WheelUp
    } else {
        MouseButton::WheelDown
    };
    if to_program(
        core,
        &hit,
        MouseEvent {
            button,
            action: MouseAction::Press,
            mods: domux_term::Mods::empty(),
            row: hit.row,
            col: hit.col,
        },
    ) {
        return;
    }
    // Copy mode's own gesture, which the keys share: it opens the mode and moves the viewport.
    // The wheel reaches it only while the keys are in a pane, because with the keys in a box
    // the reader is scrolling that box's list rather than a pane's history.
    if !core
        .model
        .client(client)
        .is_some_and(|view| matches!(view.focus, Focus::Pane(_)))
    {
        return;
    }
    let moved = core
        .panes
        .get_mut(&hit.pane)
        .is_some_and(|rt| copy_mode::scroll(rt, lines));
    if !moved {
        return;
    }
    focus_pane(core, client, &hit.pane);
    core.model.set_pane_copy_mode(&hit.pane, true);
}

/// One button event.
///
/// Only the left button acts. The other two are reported so that a binding can be given to them
/// later; nothing claims them now, and a middle-click paste would put the primary selection
/// somewhere the reader cannot see it first.
pub fn button(core: &mut Core, client: &ClientId, event: MouseEvent, count: u8) {
    if event.button != MouseButton::Left {
        return;
    }
    match core.hit_at(client, event.col, event.row) {
        Some(Hit::Pane { pane, row, col }) => {
            let hit = PaneHit { pane, row, col };
            match event.action {
                MouseAction::Press => press(core, client, &hit, count),
                MouseAction::Drag => drag(core, client, &hit),
                MouseAction::Release => release(core, client, &hit),
            }
        }
        // The chrome acts on the press. A release is the same gesture ending and a drag across
        // the chrome is not a gesture at all, so acting on either would run the operation twice
        // or run it somewhere the reader did not press.
        Some(hit) if event.action == MouseAction::Press => chrome(core, client, hit),
        _ => {}
    }
}

/// A press on the chrome runs the same handler the key for that operation runs, so a click, a
/// key and a CLI subcommand cannot mean three different things (decision 0014). A refusal from
/// one of them reaches the hint row the way a key's refusal does.
fn chrome(core: &mut Core, client: &ClientId, hit: Hit) {
    let method = match hit {
        Hit::Tab(tab) => Method::TabSelect(TabSelectParams {
            tab: tab.to_string(),
            client: Some(client.clone()),
        }),
        Hit::NewTab => Method::TabCreate(TabCreateParams {
            workspace: None,
            client: Some(client.clone()),
            cwd: None,
            name: None,
        }),
        // A click on a workspace row switches to it, which is what Enter on that row does. The
        // row is the whole gesture: nothing here moves a cursor first, because the switch is
        // what the reader asked for and the cursor follows the workspace they land in.
        Hit::Workspace(workspace) => Method::WorkspaceFocus(WorkspaceFocusParams {
            workspace: workspace.to_string(),
            client: Some(client.clone()),
        }),
        Hit::Pane { .. } => return,
    };
    if let Err(e) = core.dispatch_from_key(method, Some(client.clone())) {
        core.set_pill(Some(client), e.message, false);
    }
}

/// A press focuses the pane it landed in and starts a selection there without opening copy
/// mode: a click that never moves is a click, and it would be wrong for it to leave the pane in
/// a mode the reader did not ask for. The drag that follows is what opens it.
fn press(core: &mut Core, client: &ClientId, hit: &PaneHit, count: u8) {
    focus_pane(core, client, &hit.pane);
    match count {
        2 => return select_and_copy(core, client, hit, Granularity::Word),
        3 => return select_and_copy(core, client, hit, Granularity::Line),
        _ => {}
    }
    let Some(rt) = core.panes.get_mut(&hit.pane) else {
        return;
    };
    // A press inside a pane already in copy mode puts the copy cursor where it landed and
    // drops the selection that was there, which is the same thing the press starts anywhere
    // else: one press, one new selection.
    if let Some(copy) = rt.copy.as_mut() {
        copy.cursor = (hit.row, hit.col);
        copy.anchor = None;
        rt.dirty = true;
    }
    rt.pressed_at = Some((hit.row, hit.col));
}

/// A drag opens copy mode if the press did not, anchors the selection at the press, and moves
/// the copy cursor to the cell under the pointer.
///
/// A drag that leaves the pane clamps to its edge rather than scrolling the viewport under it, so
/// a selection made with the pointer alone reaches no further than the screen. Copy mode's keys
/// extend it into the scrollback from there, and decision 0014 records the gap.
fn drag(core: &mut Core, client: &ClientId, hit: &PaneHit) {
    let Some(rt) = core.panes.get_mut(&hit.pane) else {
        return;
    };
    let Some(pressed) = rt.pressed_at else {
        // The press happened somewhere else, so this drag is not this pane's gesture.
        return;
    };
    let history = copy_mode::history(rt);
    if rt.copy.is_none() {
        let size = rt.emulator.size();
        rt.copy = Some(CopyMode::new(size, pressed));
        core.model.set_pane_copy_mode(&hit.pane, true);
        focus_pane(core, client, &hit.pane);
    }
    let Some(rt) = core.panes.get_mut(&hit.pane) else {
        return;
    };
    let scrollback = rt.emulator.scrollback_len();
    let Some(copy) = rt.copy.as_mut() else {
        return;
    };
    if copy.anchor.is_none() {
        // The anchor is absolute, so the selection stays over the same text when the viewport
        // moves under it.
        copy.cursor = pressed;
        copy.anchor = Some(copy.abs_cursor(scrollback));
    }
    copy.cursor = (hit.row, hit.col);
    copy.offset = copy.offset.min(history);
    rt.dirty = true;
}

/// A release copies what the drag selected and leaves copy mode, which is the whole gesture in
/// one movement (decision 0014). A press that never dragged selected nothing, so it opens the
/// link under it if there is one (decision record 0020), and otherwise leaves the pane with
/// the focus the press gave it.
fn release(core: &mut Core, client: &ClientId, hit: &PaneHit) {
    let Some(rt) = core.panes.get_mut(&hit.pane) else {
        return;
    };
    let pressed = rt.pressed_at.take();
    let dragged = rt.copy.as_ref().is_some_and(|c| c.anchor.is_some());
    if !dragged {
        // The cell the button went down on, and only when the button came up on it too. A
        // press that moved without producing a drag report is not a click on anything in
        // particular, and opening the cell it happened to end on would act on a link the
        // reader never pressed.
        if pressed == Some((hit.row, hit.col)) {
            open_link(core, client, hit);
        }
        return;
    }
    let outcome = copy_mode::yank(rt);
    crate::input::finish_copy(core, client, &hit.pane, outcome);
}

/// Opens the link under a click, if the cell has one. Nothing is said when it does not: a
/// click with no link is how the reader moves the focus, and every click answering with a row
/// about what is not under it would be noise.
///
/// `link::at` decides what may be opened; this only asks. The opener runs as a job, because
/// it starts a process and the core task starts none (decision record 0006).
fn open_link(core: &mut Core, client: &ClientId, hit: &PaneHit) {
    let cwd = core.model.pane(&hit.pane).map(|p| p.cwd.clone());
    let Some(rt) = core.panes.get_mut(&hit.pane) else {
        return;
    };
    // Copy mode may be holding the viewport above the live screen, so the row the reader
    // clicked is resolved against the same offset the grid was drawn at.
    let offset = rt.copy.as_ref().map(|c| c.offset).unwrap_or(0);
    // The directory the shell reported with OSC 7 is where it really is now; the pane's
    // recorded cwd is where it started. A relative path is read against the first when there
    // is one.
    let here = rt.emulator.cwd().or(cwd);
    let Some(found) = crate::link::at(&mut rt.emulator, offset, here.as_deref(), hit.row, hit.col)
    else {
        return;
    };
    core.queue_job(
        crate::core::CoreJob::OpenLink {
            target: found.target(),
            said: found.said(),
            opener: core.deps.opener.clone(),
        },
        Some(client.clone()),
    );
}

/// What a repeated press selects.
enum Granularity {
    Word,
    Line,
}

/// A double or triple click: select the word or the line under the pointer, copy it, and leave.
/// The same one movement a drag is, with the two ends chosen by the emulator's own idea of a
/// word and of a line rather than by the pointer.
fn select_and_copy(core: &mut Core, client: &ClientId, hit: &PaneHit, what: Granularity) {
    let Some(rt) = core.panes.get_mut(&hit.pane) else {
        return;
    };
    // The grid is the viewport as it is drawn, which is what the reader clicked on. Copy mode
    // may be holding it above the live screen, so the row is resolved against the same offset.
    let offset = rt.copy.as_ref().map(|c| c.offset).unwrap_or(0);
    let scrollback = rt.emulator.scrollback_len();
    let top = scrollback.saturating_sub(offset);
    let abs_row = top + hit.row as usize;
    let (start, end) = match what {
        Granularity::Word => {
            let (first, last) = word_at(&rt.grid, hit.row, hit.col);
            (
                ScrollbackPos {
                    row: abs_row,
                    col: first,
                },
                ScrollbackPos {
                    row: abs_row,
                    col: last,
                },
            )
        }
        Granularity::Line => {
            let (first, last) = rt.emulator.logical_line(abs_row);
            let cols = rt.emulator.size().cols.saturating_sub(1);
            (
                ScrollbackPos { row: first, col: 0 },
                ScrollbackPos {
                    row: last,
                    col: cols,
                },
            )
        }
    };
    let outcome = copy_mode::yank_range(rt, start, end);
    crate::input::finish_copy(core, client, &hit.pane, outcome);
}

/// The word under a cell, as the first and last column of it. A cell that is itself a boundary
/// is its own word, so a double click on a space copies the space rather than the line around
/// it: the reader pointed at something, and every gesture answers with what was pointed at.
fn word_at(grid: &domux_term::Grid, row: u16, col: u16) -> (u16, u16) {
    let cols = grid.size().cols;
    if cols == 0 || row >= grid.size().rows {
        return (col, col);
    }
    let boundary = |c: u16| {
        let cell = grid.cell(row, c);
        // A continuation cell of a wide grapheme holds no text of its own and is part of the
        // grapheme before it, so it is never a boundary.
        if cell.width == 0 {
            return false;
        }
        let text = cell.text.as_str();
        text.is_empty()
            || text
                .chars()
                .all(|ch| ch.is_whitespace() || WORD_BOUNDARIES.contains(&ch))
    };
    let col = col.min(cols - 1);
    if boundary(col) {
        return (col, col);
    }
    let mut first = col;
    while first > 0 && !boundary(first - 1) {
        first -= 1;
    }
    let mut last = col;
    while last + 1 < cols && !boundary(last + 1) {
        last += 1;
    }
    (first, last)
}

/// Hands the event to the pane's program when the program asked for the mouse. `true` means it
/// was the program's and nothing else may act on it.
fn to_program(core: &mut Core, hit: &PaneHit, event: MouseEvent) -> bool {
    let Some(rt) = core.panes.get_mut(&hit.pane) else {
        return false;
    };
    if !rt.emulator.mode_active(Mode::MouseTracking) {
        return false;
    }
    let mut out = Vec::new();
    rt.emulator.encode_mouse(&event, &mut out);
    if out.is_empty() {
        return false;
    }
    rt.write(&out);
    true
}

/// Focuses the pane and puts this client's keys in it, through the two handlers that own those
/// questions rather than through a second copy of them: `pane.focus` moves the tab's focus, and
/// `focus.pane` is what hands the keys back from a box.
fn focus_pane(core: &mut Core, client: &ClientId, pane: &PaneId) {
    let already = core
        .model
        .client(client)
        .is_some_and(|view| matches!(view.focus, Focus::Pane(_)))
        && core.focused_pane(client).as_ref() == Some(pane);
    if already {
        return;
    }
    let _ = core.dispatch_from_key(
        Method::PaneFocus(PaneTargetParams {
            pane: Some(pane.to_string()),
            client: Some(client.clone()),
        }),
        Some(client.clone()),
    );
    let _ = core.dispatch_from_key(
        Method::FocusPane(ClientParams {
            client: Some(client.clone()),
        }),
        Some(client.clone()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_term::{Cell, Grid, Size};

    fn grid_of(line: &str) -> Grid {
        let mut g = Grid::new(Size {
            cols: line.chars().count() as u16,
            rows: 1,
        });
        for (i, ch) in line.chars().enumerate() {
            *g.cell_mut(0, i as u16) = Cell {
                text: ch.to_string().into(),
                ..Cell::default()
            };
        }
        g
    }

    #[test]
    fn word_at_takes_the_run_between_boundaries() {
        let g = grid_of("one two three");
        assert_eq!(word_at(&g, 0, 0), (0, 2), "the first word from its start");
        assert_eq!(word_at(&g, 0, 2), (0, 2), "and from its end");
        assert_eq!(word_at(&g, 0, 5), (4, 6), "the middle word");
        assert_eq!(word_at(&g, 0, 12), (8, 12), "the last word");
    }

    /// A path is one word, which is the gesture's whole point: a double click on it copies it
    /// whole. Ghostty's boundaries put `:` outside a word and `/` inside one, and domux2 uses
    /// the same set so the two agree.
    #[test]
    fn word_at_keeps_a_path_whole_and_stops_at_a_colon() {
        let g = grid_of("see src/main.rs:12 now");
        assert_eq!(word_at(&g, 0, 6), (4, 14), "src/main.rs, without the colon");
        assert_eq!(word_at(&g, 0, 16), (16, 17), "the line number after it");
    }

    #[test]
    fn word_at_on_a_boundary_is_that_cell_alone() {
        let g = grid_of("one two");
        assert_eq!(word_at(&g, 0, 3), (3, 3), "the space between the two words");
        let g = grid_of("a(b)");
        assert_eq!(word_at(&g, 0, 1), (1, 1), "the paren");
    }

    #[test]
    fn word_at_clamps_a_column_past_the_row() {
        let g = grid_of("word");
        assert_eq!(word_at(&g, 0, 99), (0, 3));
        assert_eq!(word_at(&g, 9, 0), (0, 0), "a row that is not there");
    }
}
