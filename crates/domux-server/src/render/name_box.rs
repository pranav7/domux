//! The name box: a small overlay titled with the workspace's handle, so you always know
//! which slot you are naming (interface spec 7.1).
//!
//! The title is the whole point of the box. `leader N` opens it on the workspace the client
//! is in and `n` opens it on the row under the cursor, so the two reach one operation and
//! the reader still sees which of them answered.
//!
//! It is centred with M1's `frame` geometry rather than a list overlay's: interface spec
//! 12.13 sizes the boxes that hold a list, and this one holds a line of text.

use crate::render::boxed::put_within;
use crate::render::{overlay, theme, RenderInput};
use domux_core::ids::WorkspaceId;
use domux_core::text::{display_width, sanitize_for_display, truncate_with_ellipsis};
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};

/// Room for a name and for the hint line under it, inside a border: the input on the first
/// inner row, a blank row, then the hints. `overlay::centred_area` narrows both on a screen
/// that has less.
const WIDTH: u16 = 58;
const HEIGHT: u16 = 5;

/// The row the hints sit on, counted from the top of the inside.
const HINTS_ROW: u16 = 2;

pub fn draw(input: &RenderInput, workspace: &WorkspaceId, buf: &mut Buffer) {
    // A workspace the model no longer holds draws nothing at all. A box titled `Name ` would
    // ask the reader to name something it cannot say the name of, which is the one thing this
    // overlay exists to avoid.
    let Some(w) = input.model.workspace(workspace) else {
        return;
    };
    let area = overlay::centred_area(WIDTH, HEIGHT, buf);
    if area.width == 0 || area.height == 0 {
        return;
    }
    // `frame_at` clears the area first, so the switcher this box opens over is covered rather
    // than showing through it (interface spec 12.7).
    let inner = overlay::frame_at(&format!("Name {}", w.handle), area, buf);
    let last_x = inner.x + inner.width.saturating_sub(1);
    let text = Style::default().fg(theme::TEXT).bg(theme::BASE);
    let key = Style::default().fg(theme::BLUE).bg(theme::BASE);
    let quiet = Style::default().fg(theme::OVERLAY0).bg(theme::BASE);
    // One cell of padding inside the border on each side, the same as a title's.
    let budget = inner.width.saturating_sub(2) as usize;
    if inner.height > 0 {
        let field = input.view.input.text.as_str();
        let cursor = input.view.input.cursor;
        let before: String = field.chars().take(cursor).collect();
        // The caret reverses the cell under it, so it reads without colour, the same way the
        // tab prompt's does. At the end of the name there is no character to reverse, and
        // neither is there over one a terminal cannot draw, so it takes a space.
        let after: String = field.chars().skip(cursor).collect();
        let caret = after
            .chars()
            .next()
            .map(|c| sanitize_for_display(&c.to_string()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| " ".to_string());
        let rest: String = after.chars().skip(1).collect();
        // A name wider than the box is cut at the border, which is where M1's tab prompt cuts
        // one too. The caret then goes with the cut, so a name longer than the box is wide
        // stops showing what is being typed. Said here rather than left silent: it is the
        // limit of a 58 cell box and not an oversight, and the fix if it ever bites is a
        // window that follows the caret rather than a wider box.
        let mut cx = put_within(buf, inner.x + 1, inner.y, last_x, &before, text);
        cx = put_within(
            buf,
            cx,
            inner.y,
            last_x,
            &caret,
            text.add_modifier(Modifier::REVERSED),
        );
        put_within(buf, cx, inner.y, last_x, &rest, text);
    }
    // The last result takes the hint row, the same way it takes the switcher's footer and the
    // sidebar's hint row (interface spec 12.12). A refused name leaves the box open, so the
    // answer to the key just pressed belongs where the reader is already looking.
    if inner.height > HINTS_ROW {
        match &input.view.pill {
            Some(pill) => {
                put_within(
                    buf,
                    inner.x + 1,
                    inner.y + HINTS_ROW,
                    last_x,
                    &truncate_with_ellipsis(&pill.text, budget),
                    Style::default()
                        .fg(theme::BASE)
                        .bg(if pill.ok { theme::GREEN } else { theme::RED })
                        .add_modifier(Modifier::BOLD),
                );
            }
            None => {
                // One budget for the whole line, spent piece by piece: giving each piece the
                // line's full width would let five short pieces measure as fitting and draw
                // past the border, where `put_within` would clip them with nothing to say it
                // had. `render::confirm` spends its lines by the same rule.
                let mut left = budget;
                let mut cx = inner.x + 1;
                for (piece, style) in [
                    ("⏎", key),
                    (" save    ", text),
                    ("esc", key),
                    (" cancel    ", text),
                    ("an empty name clears it", quiet),
                ] {
                    if left == 0 {
                        break;
                    }
                    let piece = truncate_with_ellipsis(piece, left);
                    left = left.saturating_sub(display_width(&piece));
                    cx = put_within(buf, cx, inner.y + HINTS_ROW, last_x, &piece, style);
                }
            }
        }
    }
    // The screen behind reads as being behind it (interface spec 7.1). After the drawing, so
    // the box itself is what `keep` keeps rather than something the box then undoes.
    overlay::dim(buf, &[area]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::FactRegistry;
    use domux_core::ids::{ClientId, PaneId, ProjectId, TabId};
    use domux_core::keymap::Keymap;
    use domux_core::model::{
        ClientView, Focus, Model, Overlay, Pill, Project, ProjectKind, TextInput, Workspace,
        WorkspaceHandle,
    };
    use domux_core::proto::Capabilities;
    use domux_term::Size;
    use ratatui::layout::Rect;
    use std::collections::HashMap;

    /// One project with `main` and one slot, the slot carrying `name`.
    fn model_with(name: Option<&str>) -> Model {
        let mut model = Model::new(1);
        let workspace = |handle: WorkspaceHandle, id: &str, name: Option<&str>| Workspace {
            id: WorkspaceId(id.into()),
            handle,
            name: name.map(str::to_string),
            path: "/p".into(),
            tabs: Vec::new(),
            last_tab: None,
        };
        model.projects.push(Project {
            id: ProjectId("p_1".into()),
            name: "proj".into(),
            root: "/p".into(),
            kind: ProjectKind::Folder,
            workspaces: vec![
                workspace(WorkspaceHandle::Main, "w_main", None),
                workspace(WorkspaceHandle::Slot(1), "w_1", name),
            ],
        });
        model
    }

    fn view(input: TextInput) -> ClientView {
        ClientView {
            id: ClientId("c_0001".into()),
            size: Size { cols: 80, rows: 24 },
            caps: Capabilities::default(),
            workspace: WorkspaceId("w_main".into()),
            tab: TabId("t_0001".into()),
            focus: Focus::Pane(PaneId("p_0001".into())),
            sidebar_open: false,
            sidebar_forced: false,
            overlay: Some(Overlay::NameWorkspace(WorkspaceId("w_1".into()))),
            chord: None,
            filter: String::new(),
            last_active_seq: 0,
            projects_cursor: None,
            projects_scroll: 0,
            filtering: false,
            input,
            overlay_under: None,
            pill: None,
        }
    }

    /// Draws into a buffer already holding `fill`, so a test can tell what the box covered
    /// from what it merely drew over nothing.
    fn draw_over(model: &Model, view: &ClientView, cols: u16, rows: u16, fill: &str) -> Buffer {
        let Some(Overlay::NameWorkspace(target)) = view.overlay.clone() else {
            panic!("the fixture's view has the name box open");
        };
        let panes = HashMap::new();
        let facts = FactRegistry::new();
        let keymap = Keymap::defaults();
        let input = RenderInput {
            model,
            facts: &facts,
            panes: &panes,
            view,
            keymap: &keymap,
            now: chrono::Local::now(),
            config_error: None,
            hint: None,
            notes: &[],
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, cols, rows));
        for y in 0..rows {
            for x in 0..cols {
                buf[(x, y)].set_symbol(fill);
            }
        }
        draw(&input, &target, &mut buf);
        buf
    }

    fn line(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// The box covers what was under it rather than drawing into it.
    ///
    /// `@` is the marker because nothing **this fixture** draws can emit one: the project is
    /// `proj` at `/p`, the handles are `main` and `workspace-1`, there are no facts, and the
    /// hints are domux's own words. It is not a character the box could never draw - a name
    /// or a path with an `@` in it would put one on the screen.
    #[test]
    fn the_box_covers_what_was_under_it() {
        let model = model_with(None);
        let buf = draw_over(&model, &view(TextInput::new("")), 80, 24, "@");
        // 58 wide and 5 tall, centred over the workpanel: columns 11 to 68, rows 10 to 14.
        for y in 10..15u16 {
            for x in 11..69u16 {
                assert_ne!(buf[(x, y)].symbol(), "@", "the box left {x},{y} showing");
            }
        }
        assert_eq!(
            buf[(10u16, 12u16)].symbol(),
            "@",
            "and covered nothing else"
        );
        assert_eq!(buf[(69u16, 12u16)].symbol(), "@");
        assert_eq!(buf[(40u16, 9u16)].symbol(), "@");
        assert_eq!(buf[(40u16, 15u16)].symbol(), "@");
    }

    /// The title names the slot being named, and the hints say what the two keys do.
    #[test]
    fn the_title_carries_the_handle_and_the_hints_say_what_enter_and_esc_do() {
        let model = model_with(None);
        let buf = draw_over(&model, &view(TextInput::new("")), 80, 24, " ");
        assert!(
            line(&buf, 10).contains("┌ Name workspace-1 "),
            "{}",
            line(&buf, 10)
        );
        assert!(
            line(&buf, 13).contains("⏎ save    esc cancel    an empty name clears it"),
            "{}",
            line(&buf, 13)
        );
    }

    /// The caret sits where the cursor is, not at the end of the text.
    #[test]
    fn the_caret_reverses_the_cell_the_cursor_is_on() {
        let model = model_with(None);
        let mut input = TextInput::new("auth");
        input.left();
        input.left();
        let buf = draw_over(&model, &view(input), 80, 24, " ");
        assert!(line(&buf, 11).contains("│ auth"), "{}", line(&buf, 11));
        // The inside starts at column 12 and the text one cell in, so `a` is at 13 and the
        // caret is on `t`, the third character.
        assert!(
            !buf[(14u16, 11u16)].modifier.contains(Modifier::REVERSED),
            "the cell before the cursor is not the caret"
        );
        assert!(
            buf[(15u16, 11u16)].modifier.contains(Modifier::REVERSED),
            "the cell the cursor is on is"
        );
        assert!(
            !buf[(16u16, 11u16)].modifier.contains(Modifier::REVERSED),
            "and the cell after it is not"
        );
    }

    /// A refused name leaves the box open, so its message takes the hint row rather than the
    /// row of keys the reader already knows (interface spec 12.12).
    #[test]
    fn a_pill_takes_the_hint_row_while_one_is_showing() {
        let model = model_with(None);
        let mut v = view(TextInput::new("workspace-2"));
        v.pill = Some(Pill {
            text: "workspace-2 is a handle; pick another".into(),
            ok: false,
            at: "2026-09-04T14:32:00+01:00".into(),
        });
        let buf = draw_over(&model, &v, 80, 24, " ");
        assert!(
            line(&buf, 13).contains("workspace-2 is a handle; pick another"),
            "{}",
            line(&buf, 13)
        );
        assert!(
            !line(&buf, 13).contains("esc cancel"),
            "the keys give the row up while the answer is on it: {}",
            line(&buf, 13)
        );
        assert_eq!(
            buf[(13u16, 13u16)].bg,
            theme::RED,
            "refused reads as refused"
        );
    }

    /// A workspace the model no longer holds draws nothing, rather than a box with no handle
    /// in its title.
    #[test]
    fn a_workspace_that_is_gone_draws_no_box() {
        let model = model_with(None);
        let mut v = view(TextInput::new(""));
        v.overlay = Some(Overlay::NameWorkspace(WorkspaceId("w_gone".into())));
        let buf = draw_over(&model, &v, 80, 24, "@");
        assert_eq!(line(&buf, 10), "@".repeat(80), "{}", line(&buf, 10));
    }

    /// The screen around the box is dimmed, and the box is not.
    #[test]
    fn the_screen_outside_the_box_is_dimmed() {
        let model = model_with(None);
        let buf = draw_over(&model, &view(TextInput::new("")), 80, 24, " ");
        assert!(buf[(0u16, 0u16)].modifier.contains(Modifier::DIM));
        assert!(!buf[(11u16, 10u16)].modifier.contains(Modifier::DIM));
    }

    /// The box is centred over the workpanel, so the top bar stays readable under it
    /// (the reason `overlay::centred_area` exists).
    #[test]
    fn the_box_leaves_the_top_bar_alone() {
        let model = model_with(None);
        let buf = draw_over(&model, &view(TextInput::new("")), 80, 24, "@");
        assert_eq!(line(&buf, 0), "@".repeat(80));
    }
}
