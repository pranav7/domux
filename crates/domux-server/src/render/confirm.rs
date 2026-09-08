//! The confirmation overlay: the question a destructive key asks before it acts
//! (interface spec 7.3). Task 16 writes the project kind's copy and the drawing they share;
//! Task 18 adds the workspace kinds' copy to `lines`.
//!
//! `ConfirmKind::CloseTab` reaches here too, and must stay drawing nothing: M1 asks that
//! question in the tab's own cell (`top_bar::draw`), so a box drawn here would ask it twice.

use crate::render::boxed::put_within;
use crate::render::{overlay, theme, RenderInput};
use domux_core::model::ConfirmKind;
use domux_core::text::{display_width, truncate_with_ellipsis};
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// A cell of padding inside the border on each side, and the two border columns.
const CHROME: u16 = 4;

/// What the confirmation says, in the order it is drawn: the title goes in the border and
/// the rest fills the box.
struct Question {
    title: String,
    lines: Vec<Line<'static>>,
}

pub fn draw(input: &RenderInput, kind: &ConfirmKind, buf: &mut Buffer) {
    let Some(question) = question(input, kind) else {
        return;
    };
    let widest = question
        .lines
        .iter()
        .map(|l| l.width())
        .chain(std::iter::once(display_width(&question.title)))
        .max()
        .unwrap_or(0) as u16;
    let area = overlay::centred_area(
        widest.saturating_add(CHROME),
        question.lines.len() as u16 + 2,
        buf,
    );
    if area.width == 0 || area.height == 0 {
        return;
    }
    // An empty title, then the question written into the border row here: `Boxed` draws a
    // title in the accent when focused and in `overlay1` when not, and this one is red.
    let inner = overlay::frame_at("", area, buf);
    let right = area.x + area.width - 1;
    put_within(
        buf,
        area.x + 1,
        area.y,
        right,
        &format!(
            " {} ",
            truncate_with_ellipsis(&question.title, area.width.saturating_sub(CHROME) as usize)
        ),
        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
    );
    let width = inner.width.saturating_sub(2) as usize;
    let last_x = inner.x + inner.width.saturating_sub(1);
    for (i, line) in question.lines.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let mut x = inner.x + 1;
        // One budget for the whole line, spent span by span: giving each span the line's
        // full width would let three short spans measure as fitting and draw past the
        // border, which `put_within` would then clip without an ellipsis to say so.
        //
        // Unpinned, and it cannot be pinned by any fixture here: the box is sized from its
        // widest line, so no line reaches the boundary until `centred_area` clamps the box
        // to a screen narrower than its content. Only the keys line has several spans, and
        // the two versions then differ by one cell - the ellipsis - because `put_within`
        // clips both at the same column. Said here rather than left silent.
        let mut budget = width;
        for span in &line.spans {
            if budget == 0 {
                break;
            }
            let text = truncate_with_ellipsis(&span.content, budget);
            budget = budget.saturating_sub(display_width(&text));
            x = put_within(buf, x, inner.y + i as u16, last_x, &text, span.style);
        }
    }
    // The screen behind reads as being behind it (interface spec 7.1). After the drawing,
    // so the box itself is what `keep` keeps rather than something the box then undoes.
    overlay::dim(buf, &[area]);
}

/// The copy for one question, or `None` when there is nothing to ask about: a project the
/// model no longer holds, or the tab kind M1 asks about somewhere else.
fn question(input: &RenderInput, kind: &ConfirmKind) -> Option<Question> {
    match kind {
        // M1's, asked in the tab's own cell.
        ConfirmKind::CloseTab(_) => None,
        ConfirmKind::RemoveProject(id) => {
            let project = input.model.project(id)?;
            let copy = crate::api::project::removal_copy(
                &project.name,
                project.workspaces.len(),
                &project.root,
            );
            Some(Question {
                title: copy.title,
                lines: vec![
                    Line::from(Span::styled(
                        copy.identity,
                        Style::default().fg(theme::OVERLAY1),
                    )),
                    Line::default(),
                    Line::from(Span::styled(copy.removes, Style::default().fg(theme::TEXT))),
                    Line::from(Span::styled(
                        crate::api::project::KEEPS_THE_FOLDER.to_string(),
                        Style::default().fg(theme::OVERLAY0),
                    )),
                    Line::default(),
                    keys("y", "remove project", "esc", "keep project"),
                ],
            })
        }
        // Task 18's, with `workspace.delete`.
        ConfirmKind::DeleteWorkspace(_) => None,
    }
}

/// `y remove project    esc keep project`: the key in blue, what it does in `text`, four
/// spaces between the pair (interface spec 7.3).
fn keys(yes: &str, does: &str, no: &str, undoes: &str) -> Line<'static> {
    let key = Style::default().fg(theme::BLUE);
    let word = Style::default().fg(theme::TEXT);
    Line::from(vec![
        Span::styled(yes.to_string(), key),
        Span::styled(format!(" {does}    "), word),
        Span::styled(no.to_string(), key),
        Span::styled(format!(" {undoes}"), word),
    ])
}
