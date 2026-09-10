//! Drawing the toast: a box in the bottom right of the workpanel (interface spec 8.1).
//!
//! It is inset from the corner rather than pressed into it, so it reads as a thing on top of
//! the panes and not as a piece of the frame. It never covers the sidebar, because it is
//! placed inside the workpanel, and it never covers the row a pane box would put its bottom
//! rule on.

use crate::render::boxed::{put_within, Boxed};
use crate::render::{theme, to_rect, RenderInput};
use domux_core::text::wrap_to_width;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// The widest a toast gets, however wide the screen is. A line of text past this is harder to
/// read, not easier (interface spec 8.1).
pub const MAX_WIDTH: u16 = 60;
/// The gap between the box and the two edges of the workpanel it sits in.
pub const INSET_X: u16 = 2;
pub const INSET_Y: u16 = 1;
/// One cell of air inside the border on each side, so the text does not touch the rule.
const PAD_X: u16 = 1;

/// The text a toast of this width holds, wrapped. Public because the height depends on it and
/// `area_for` needs the count before anything is drawn.
pub fn lines_for(toast: &crate::toast::Toast, width: u16) -> Vec<String> {
    let budget = width.saturating_sub(2 + PAD_X * 2) as usize;
    if budget == 0 {
        return Vec::new();
    }
    toast
        .lines
        .iter()
        .flat_map(|line| wrap_to_width(line, budget))
        .collect()
}

/// How wide a toast is in this workpanel: as wide as it likes, until the workpanel is too
/// narrow to inset it, when it takes what is left.
pub fn width_for(workpanel: Rect) -> u16 {
    MAX_WIDTH.min(workpanel.width.saturating_sub(INSET_X * 2))
}

/// The box, in the bottom right corner of `workpanel`. A workpanel with no room for the box
/// and its inset answers an empty rectangle and nothing is drawn: a toast is the least of
/// what a screen that small has to fit (principle 6).
pub fn area_for(workpanel: Rect, lines: usize) -> Rect {
    let width = width_for(workpanel);
    let height = lines as u16 + 2;
    let room_across = workpanel.width >= width + INSET_X * 2 && width > 2 + PAD_X * 2;
    let room_down = workpanel.height >= height + INSET_Y * 2;
    if !room_across || !room_down || lines == 0 {
        return Rect::new(workpanel.x, workpanel.y, 0, 0);
    }
    Rect::new(
        workpanel.x + workpanel.width - INSET_X - width,
        workpanel.y + workpanel.height - INSET_Y - height,
        width,
        height,
    )
}

pub fn draw(input: &RenderInput, buf: &mut Buffer) {
    let Some(toast) = input.toast else { return };
    let workpanel = to_rect(crate::render::workpanel_area(input.view));
    let lines = lines_for(toast, width_for(workpanel));
    let area = area_for(workpanel, lines.len());
    if area.width == 0 || area.height == 0 {
        return;
    }
    // The box covers what is under it rather than letting a pane's text show through its
    // padding, so every cell is written before the border goes on.
    let ground = Style::default().bg(theme::MANTLE).fg(theme::TEXT);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf[(x, y)].reset();
            buf[(x, y)].set_style(ground);
        }
    }
    let inner = Boxed {
        title: "",
        flag: None,
        focused: false,
        bottom_rule: true,
    }
    .render(area, buf);
    let last_x = inner.x + inner.width.saturating_sub(1);
    for (i, line) in lines.iter().enumerate() {
        // The first line is the headline and the rest is what the reader needs after it, so
        // the two are told apart by weight rather than by a blank row this box has no space
        // for.
        let style = if i == 0 {
            Style::default().fg(theme::TEXT).bg(theme::MANTLE)
        } else {
            Style::default().fg(theme::SUBTEXT0).bg(theme::MANTLE)
        };
        put_within(
            buf,
            inner.x + PAD_X,
            inner.y + i as u16,
            last_x,
            line,
            style,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toast::Toast;
    use crate::FixedClock;

    fn toast(lines: &[&str]) -> Toast {
        let mut t = Toast::new(lines[0], FixedClock::at("2026-09-10T14:32:00").0);
        for line in &lines[1..] {
            t = t.and(*line);
        }
        t
    }

    #[test]
    fn the_box_sits_in_the_bottom_right_corner_inset_from_both_edges() {
        let workpanel = Rect::new(0, 1, 100, 23);
        let area = area_for(workpanel, 1);
        assert_eq!(area.width, MAX_WIDTH);
        assert_eq!(area.height, 3, "a rule, the line, a rule");
        assert_eq!(
            area.x + area.width,
            workpanel.x + workpanel.width - INSET_X,
            "two cells of air at the right edge"
        );
        assert_eq!(
            area.y + area.height,
            workpanel.y + workpanel.height - INSET_Y,
            "one row of air at the bottom"
        );
    }

    /// The sidebar's own columns are not the workpanel's, so a toast placed in the workpanel
    /// cannot reach them however wide it is (interface spec 8.1).
    #[test]
    fn the_box_stays_out_of_the_columns_left_of_the_workpanel() {
        let workpanel = Rect::new(31, 1, 69, 23);
        let area = area_for(workpanel, 1);
        assert!(area.x >= workpanel.x, "{area:?}");
    }

    #[test]
    fn a_narrow_workpanel_gives_the_box_what_is_left_of_it() {
        let area = area_for(Rect::new(0, 1, 40, 23), 1);
        assert_eq!(area.width, 36, "the width less its two insets");
    }

    #[test]
    fn a_workpanel_with_no_room_for_the_box_draws_none() {
        assert_eq!(area_for(Rect::new(0, 1, 100, 3), 1).height, 0);
        assert_eq!(area_for(Rect::new(0, 1, 6, 23), 1).width, 0);
        assert_eq!(area_for(Rect::new(0, 1, 100, 23), 0).width, 0);
    }

    #[test]
    fn a_line_too_long_for_the_box_is_wrapped_and_not_cut() {
        let long = "the machine is held awake, but the lid is not: sudo said no. Run the install command to set full mode up";
        let lines = lines_for(&toast(&["Stay awake turned on", long]), MAX_WIDTH);
        assert_eq!(lines[0], "Stay awake turned on");
        assert!(lines.len() > 2, "{lines:?}");
        let budget = (MAX_WIDTH - 4) as usize;
        for line in &lines {
            assert!(domux_core::text::display_width(line) <= budget, "{line}");
        }
        assert!(
            lines[1..].join(" ").contains("set full mode up"),
            "every word survives the wrap: {lines:?}"
        );
    }
}
