//! Cell, grid, and cursor types shared by every emulator implementation and the renderer.

use bitflags::bitflags;
use compact_str::CompactString;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// A cell color as the inner program set it. `Default` means the outer terminal's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(Rgb),
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Attrs: u16 {
        const BOLD = 1 << 0;
        const DIM = 1 << 1;
        const ITALIC = 1 << 2;
        const UNDERLINE = 1 << 3;
        const DOUBLE_UNDERLINE = 1 << 4;
        const CURLY_UNDERLINE = 1 << 5;
        const DOTTED_UNDERLINE = 1 << 6;
        const DASHED_UNDERLINE = 1 << 7;
        const BLINK = 1 << 8;
        const INVERSE = 1 << 9;
        const INVISIBLE = 1 << 10;
        const STRIKETHROUGH = 1 << 11;
        const OVERLINE = 1 << 12;
    }
}

/// One terminal cell. `width` is 2 for the left half of a wide grapheme, 0 for the right
/// half (the spacer, whose `text` is empty), and 1 otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: CompactString,
    pub width: u8,
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Option<Color>,
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            text: CompactString::const_new(" "),
            width: 1,
            fg: Color::Default,
            bg: Color::Default,
            underline_color: None,
            attrs: Attrs::empty(),
        }
    }
}

impl Cell {
    fn has_default_style(&self) -> bool {
        self.fg == Color::Default
            && self.bg == Color::Default
            && self.underline_color.is_none()
            && self.attrs.is_empty()
    }

    fn style_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let attr_names = [
            (Attrs::BOLD, "bold"),
            (Attrs::DIM, "dim"),
            (Attrs::ITALIC, "italic"),
            (Attrs::UNDERLINE, "underline"),
            (Attrs::DOUBLE_UNDERLINE, "double_underline"),
            (Attrs::CURLY_UNDERLINE, "curly_underline"),
            (Attrs::DOTTED_UNDERLINE, "dotted_underline"),
            (Attrs::DASHED_UNDERLINE, "dashed_underline"),
            (Attrs::BLINK, "blink"),
            (Attrs::INVERSE, "inverse"),
            (Attrs::INVISIBLE, "invisible"),
            (Attrs::STRIKETHROUGH, "strikethrough"),
            (Attrs::OVERLINE, "overline"),
        ];
        let attrs: Vec<&str> = attr_names
            .iter()
            .filter(|(a, _)| self.attrs.contains(*a))
            .map(|(_, n)| *n)
            .collect();
        if !attrs.is_empty() {
            parts.push(attrs.join(","));
        }
        if self.fg != Color::Default {
            parts.push(format!("fg={}", color_text(self.fg)));
        }
        if self.bg != Color::Default {
            parts.push(format!("bg={}", color_text(self.bg)));
        }
        if let Some(ul) = self.underline_color {
            parts.push(format!("ul={}", color_text(ul)));
        }
        parts.join(" ")
    }
}

fn color_text(c: Color) -> String {
    match c {
        Color::Default => "default".to_string(),
        Color::Indexed(i) => format!("idx{i}"),
        Color::Rgb(Rgb { r, g, b }) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
    pub shape: CursorShape,
    pub blink: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor {
            row: 0,
            col: 0,
            visible: true,
            shape: CursorShape::Block,
            blink: false,
        }
    }
}

/// The visible screen as row-major cells. Reused across snapshots to avoid allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    size: Size,
    cells: Vec<Cell>,
}

impl Grid {
    pub fn new(size: Size) -> Self {
        Grid {
            size,
            cells: vec![Cell::default(); size.cols as usize * size.rows as usize],
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn cell(&self, row: u16, col: u16) -> &Cell {
        &self.cells[row as usize * self.size.cols as usize + col as usize]
    }

    pub fn cell_mut(&mut self, row: u16, col: u16) -> &mut Cell {
        &mut self.cells[row as usize * self.size.cols as usize + col as usize]
    }

    pub fn row(&self, row: u16) -> &[Cell] {
        let start = row as usize * self.size.cols as usize;
        &self.cells[start..start + self.size.cols as usize]
    }

    /// Resizes in place, keeping the top-left content. Emulators call this before filling.
    pub fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        let mut next = Grid::new(size);
        for r in 0..self.size.rows.min(size.rows) {
            for c in 0..self.size.cols.min(size.cols) {
                *next.cell_mut(r, c) = self.cell(r, c).clone();
            }
        }
        *self = next;
    }

    /// Resets every cell to the default so an emulator can fill the grid from scratch.
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::default();
        }
    }

    /// The golden format: a header, one framed text row per grid row, then style runs.
    /// Wide graphemes print once; their spacer cells print nothing, so rows stay aligned
    /// in a terminal that measures width the same way the emulator did.
    pub fn to_text(&self, cursor: &Cursor) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "size {}x{}", self.size.cols, self.size.rows);
        let shape = match cursor.shape {
            CursorShape::Block => "block",
            CursorShape::Underline => "underline",
            CursorShape::Bar => "bar",
        };
        let _ = writeln!(
            out,
            "cursor row={} col={} visible={} shape={} blink={}",
            cursor.row, cursor.col, cursor.visible, shape, cursor.blink
        );
        for r in 0..self.size.rows {
            out.push('|');
            for cell in self.row(r) {
                if cell.width == 0 {
                    continue;
                }
                out.push_str(&cell.text);
            }
            out.push_str("|\n");
        }
        out.push_str("styles\n");
        for r in 0..self.size.rows {
            let row = self.row(r);
            let mut c = 0usize;
            while c < row.len() {
                if row[c].has_default_style() {
                    c += 1;
                    continue;
                }
                let style = row[c].style_text();
                let start = c;
                while c + 1 < row.len()
                    && !row[c + 1].has_default_style()
                    && row[c + 1].style_text() == style
                {
                    c += 1;
                }
                let _ = writeln!(out, "r{r} c{start}-{c} {style}");
                c += 1;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_with(text: &str) -> Grid {
        let mut g = Grid::new(Size { cols: 6, rows: 2 });
        for (col, ch) in text.chars().enumerate() {
            let cell = g.cell_mut(0, col as u16);
            cell.text.clear();
            cell.text.push(ch);
        }
        g
    }

    #[test]
    fn to_text_frames_rows_with_bars_and_keeps_trailing_spaces() {
        let g = grid_with("ab");
        let text = g.to_text(&Cursor::default());
        assert_eq!(
            text,
            "size 6x2\ncursor row=0 col=0 visible=true shape=block blink=false\n|ab    |\n|      |\nstyles\n"
        );
    }

    #[test]
    fn to_text_prints_a_wide_grapheme_once_and_skips_its_spacer() {
        let mut g = Grid::new(Size { cols: 6, rows: 1 });
        g.cell_mut(0, 0).text = "漢".into();
        g.cell_mut(0, 0).width = 2;
        g.cell_mut(0, 1).width = 0;
        g.cell_mut(0, 2).text = "x".into();
        let text = g.to_text(&Cursor::default());
        assert!(text.contains("|漢x   |\n"), "{text}");
    }

    #[test]
    fn to_text_lists_non_default_style_runs() {
        let mut g = grid_with("abcd");
        for col in 1..3 {
            let c = g.cell_mut(0, col);
            c.attrs = Attrs::BOLD | Attrs::UNDERLINE;
            c.fg = Color::Indexed(1);
        }
        g.cell_mut(1, 0).bg = Color::Rgb(Rgb { r: 1, g: 2, b: 3 });
        let text = g.to_text(&Cursor::default());
        assert!(
            text.ends_with("styles\nr0 c1-2 bold,underline fg=idx1\nr1 c0-0 bg=#010203\n"),
            "{text}"
        );
    }

    #[test]
    fn resize_keeps_top_left_text_and_fills_with_blank_cells() {
        let mut g = grid_with("abcd");
        g.resize(Size { cols: 3, rows: 3 });
        assert_eq!(g.size(), Size { cols: 3, rows: 3 });
        assert_eq!(g.cell(0, 0).text, "a");
        assert_eq!(g.cell(0, 2).text, "c");
        assert_eq!(g.cell(2, 2).text, " ");
    }
}
