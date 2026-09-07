//! Tab cells: `1` or `2 pr1`, the current tab filled accent, `│` separators, `+`.
//!
//! The row is built as cells and only then drawn, because it has to fit a budget. The current
//! tab is the one visible focus target (principle 2), so it is what the row keeps: when the
//! tabs are wider than the room, the row shows the widest run of tabs around the current one
//! that fits and marks each elided end with `…`, rather than dropping tabs off either end with
//! nothing to say it did (principle 6).

use crate::render::boxed::put_within;
use crate::render::theme;
use domux_core::model::{PromptKind, Tab, TextInput};
use domux_core::text::{display_width, sanitize_for_display};
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};

/// The cells an elided end takes: `…` and the separator after it.
const ELISION: usize = 2;
/// The cells the `+` takes: ` + ` and the separator after it.
const PLUS: usize = 4;

/// One cell of the row: the styled runs it draws, and the cells they take.
///
/// Measured on sanitized text, so the width is the width `put_within` will draw. A tab name is
/// typed at the prompt or passed to `tab.rename`, so it can hold a control character or a
/// zero-width grapheme, and a budget measured on cells that will not be drawn is not a budget.
struct TabCell {
    runs: Vec<(String, Style)>,
    width: usize,
}

impl TabCell {
    fn new(runs: Vec<(String, Style)>) -> TabCell {
        let runs: Vec<(String, Style)> = runs
            .into_iter()
            .map(|(text, style)| (sanitize_for_display(&text), style))
            .collect();
        let width = runs.iter().map(|(text, _)| display_width(text)).sum();
        TabCell { runs, width }
    }

    /// The cells this tab takes in the row: its own, plus the separator after it.
    fn slot(&self) -> usize {
        self.width + 1
    }
}

/// The tab row of one workspace, ready to measure and then draw.
pub struct TabRow {
    cells: Vec<TabCell>,
    current: usize,
    /// The one cell that must be visible whatever else goes: the cell the keys go to. That is the
    /// prompt's tab while a prompt is open on this row - a name being typed cannot be elided off
    /// the screen - and the current tab otherwise.
    anchor: usize,
}

impl TabRow {
    /// `pane_focus` is whether the keys go to a pane rather than to a prompt or an overlay. It
    /// decides whether the current tab's cell carries the accent fill - see `cell_for`.
    pub fn new(
        tabs: &[Tab],
        current: usize,
        prompt: Option<&PromptKind>,
        pane_focus: bool,
    ) -> TabRow {
        let cells = tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| cell_for(tab, i, current, prompt, pane_focus))
            .collect();
        let anchor = match prompt {
            Some(PromptKind::TabName { tab: named, .. }) => {
                tabs.iter().position(|t| &t.id == named).unwrap_or(current)
            }
            None => current,
        };
        TabRow {
            cells,
            current,
            anchor,
        }
    }

    /// The cells the whole row wants: every tab and its separator, then `│ + │`.
    fn natural(&self) -> usize {
        self.cells.iter().map(TabCell::slot).sum::<usize>() + PLUS
    }

    /// The cells the row keeps before the right end takes any: the anchor cell, its separator, and
    /// an elision mark at each end. Never more than the whole row wants.
    pub fn floor(&self) -> usize {
        let anchor = self
            .cells
            .get(self.anchor)
            .map(|c| c.slot() + 2 * ELISION)
            .unwrap_or(PLUS);
        self.natural().min(anchor)
    }

    /// Draws the row from `x` on row `y` into `budget` cells, and returns the x after the last
    /// cell it drew.
    pub fn draw(&self, x: u16, y: u16, budget: usize, buf: &mut Buffer) -> u16 {
        let sep = Style::default().fg(theme::SURFACE0).bg(theme::MANTLE);
        let plus_style = Style::default().fg(theme::SURFACE2).bg(theme::MANTLE);
        if budget == 0 {
            return x;
        }
        // Inclusive, and clamped into `u16` before the cast: a budget is derived from a
        // client's reported screen, which nothing clamps, and a wrapped edge would let the row
        // write over its neighbour instead of stopping at it.
        let last_x = x
            .saturating_add(budget.min(u16::MAX as usize) as u16)
            .saturating_sub(1);
        let n = self.cells.len();
        if n == 0 {
            let cx = put_within(buf, x, y, last_x, "│", sep);
            let cx = put_within(buf, cx, y, last_x, " + ", plus_style);
            return put_within(buf, cx, y, last_x, "│", sep);
        }
        let current = self.current.min(n - 1);
        let anchor = self.anchor.min(n - 1);
        let framed = |lo: usize, hi: usize, tabs: usize| {
            tabs + if lo > 0 { ELISION } else { 0 } + if hi < n { ELISION } else { 0 }
        };
        // The window of tabs the row shows. It spans the anchor and the current tab, and grows one
        // tab at a time to each side while the row still fits.
        //
        // Those two are the same cell unless a prompt names another tab, and then they are two
        // different facts - where the keys go, and which tab you are viewing - each worth a cell
        // before the `+` is. When even they do not both fit, the keys win and the elision mark
        // says the other is there.
        let mut lo = anchor.min(current);
        let mut hi = anchor.max(current) + 1;
        let mut tabs = self.cells[lo..hi].iter().map(TabCell::slot).sum::<usize>();
        if framed(lo, hi, tabs) > budget {
            lo = anchor;
            hi = anchor + 1;
            tabs = self.cells[anchor].slot();
        }
        // The `+` is reserved before the window grows, so it does not appear and disappear as
        // the current tab changes width. It goes only when a tab needs its cells.
        let plus = framed(lo, hi, tabs) + PLUS <= budget;
        let cap = budget - if plus { PLUS } else { 0 };
        loop {
            let mut grew = false;
            if hi < n && framed(lo, hi + 1, tabs + self.cells[hi].slot()) <= cap {
                tabs += self.cells[hi].slot();
                hi += 1;
                grew = true;
            }
            if lo > 0 && framed(lo - 1, hi, tabs + self.cells[lo - 1].slot()) <= cap {
                lo -= 1;
                tabs += self.cells[lo].slot();
                grew = true;
            }
            if !grew {
                break;
            }
        }
        if framed(lo, hi, tabs) > budget {
            // Not even the anchor's own cell fits. It is drawn cut at one cell short of the
            // budget and that cell is the `…`, so a cut cell says it was cut (principle 6)
            // instead of ending wherever the clip fell.
            let mut cx = x;
            for (text, style) in &self.cells[anchor].runs {
                cx = put_within(buf, cx, y, last_x.saturating_sub(1), text, *style);
            }
            return put_within(buf, cx, y, last_x, "…", sep);
        }
        let mut cx = x;
        if lo > 0 {
            cx = put_within(buf, cx, y, last_x, "…", sep);
            cx = put_within(buf, cx, y, last_x, "│", sep);
        }
        for cell in &self.cells[lo..hi] {
            for (text, style) in &cell.runs {
                cx = put_within(buf, cx, y, last_x, text, *style);
            }
            cx = put_within(buf, cx, y, last_x, "│", sep);
        }
        if hi < n {
            cx = put_within(buf, cx, y, last_x, "…", sep);
            cx = put_within(buf, cx, y, last_x, "│", sep);
        }
        if plus {
            cx = put_within(buf, cx, y, last_x, " + ", plus_style);
            cx = put_within(buf, cx, y, last_x, "│", sep);
        }
        cx
    }
}

/// One tab's cell.
///
/// The accent fill has one meaning in this grammar: your input goes here. "This is the tab you
/// are viewing" is a different fact, and both deserve to be visible, but they must not use the
/// same mark - two identical signals meaning two different things is worse than one signal
/// (principle 2, ruled 2026-09-07). So at most one run of cells on the screen is accent-filled,
/// and it is the one that owns the keys:
///
/// - keys go to a pane: the current tab is accent-filled. The focused pane box marks itself with
///   an accent *border*, which is a different treatment, as it is for every other box.
/// - keys go to a prompt: the prompt's own cell is accent-filled and the current tab is not.
/// - keys go to another overlay: nothing in the tab row is filled. The overlay marks itself the
///   way a focused box does.
///
/// Without the fill the current tab is still the bright bold cell against the dim others - the
/// location's own treatment, not a new mark.
fn cell_for(
    tab: &Tab,
    i: usize,
    current: usize,
    prompt: Option<&PromptKind>,
    pane_focus: bool,
) -> TabCell {
    // The prompt belongs to the cell of the tab it names, which is not always the current one:
    // `tab.rename` with no name can name another tab and still open the prompt in this view.
    if let Some(PromptKind::TabName { tab: named, input }) = prompt {
        if named == &tab.id {
            return prompt_cell(i + 1, input);
        }
    }
    let label = match &tab.name {
        Some(name) => format!(" {} {} ", i + 1, name),
        None => format!(" {} ", i + 1),
    };
    let style = match (i == current, pane_focus) {
        (true, true) => Style::default()
            .fg(theme::BASE)
            .bg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
        (true, false) => Style::default()
            .fg(theme::TEXT)
            .bg(theme::MANTLE)
            .add_modifier(Modifier::BOLD),
        (false, _) => Style::default().fg(theme::OVERLAY1).bg(theme::MANTLE),
    };
    TabCell::new(vec![(label, style)])
}

/// `Name tab 2 › pr1▮` (interface spec 4.7): the label at reduced weight, the name as typed,
/// and a block caret, all in the tab's own accent-filled cell.
fn prompt_cell(number: usize, input: &TextInput) -> TabCell {
    let fill = Style::default().fg(theme::BASE).bg(theme::ACCENT);
    let label = fill.add_modifier(Modifier::DIM);
    let before: String = input.text.chars().take(input.cursor).collect();
    let after: String = input.text.chars().skip(input.cursor).collect();
    // The caret: reverse the cell under it so it reads without colour. A caret at the end of
    // the name has no character under it, and neither has one over a character a terminal
    // cannot draw, so it takes a space.
    let caret = after
        .chars()
        .next()
        .map(|c| sanitize_for_display(&c.to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| " ".to_string());
    let rest: String = after.chars().skip(1).collect();
    TabCell::new(vec![
        (format!(" Name tab {number} › "), label),
        (before, fill),
        (caret, fill.add_modifier(Modifier::REVERSED)),
        (rest, fill),
        (" ".to_string(), fill),
    ])
}
