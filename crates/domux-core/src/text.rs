//! Display width in terminal cells, by grapheme, and the two helpers every row uses.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The number of terminal cells `s` occupies. Measured per grapheme cluster so a combining
/// mark adds nothing and an emoji with modifiers counts once.
pub fn display_width(s: &str) -> usize {
    s.graphemes(true).map(grapheme_width).sum()
}

fn grapheme_width(g: &str) -> usize {
    // A cluster with a variation selector or an emoji modifier is drawn as one wide glyph by
    // every terminal domux targets, so clamp such clusters to 2 cells.
    let w = UnicodeWidthStr::width(g);
    if g.chars().count() > 1 && w > 2 {
        2
    } else {
        w
    }
}

/// Shortens `s` to at most `max` cells, ending in `…` when anything was cut. Never splits a
/// grapheme; a wide grapheme that does not fit is dropped whole.
pub fn truncate_with_ellipsis(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1; // room for the ellipsis
    let mut out = String::new();
    let mut used = 0;
    for g in s.graphemes(true) {
        let w = grapheme_width(g);
        if used + w > budget {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out.push('…');
    out
}

/// Pads `s` with spaces on the right to `width` cells. Longer strings are returned unchanged.
pub fn pad(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + width - w);
    out.push_str(s);
    for _ in 0..(width - w) {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_cells_not_bytes() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("漢字"), 4);
        assert_eq!(display_width("é"), 1); // e plus combining acute
        assert_eq!(display_width("👍🏽"), 2); // emoji with a skin tone modifier is one grapheme
    }

    #[test]
    fn truncate_with_ellipsis_never_splits_a_grapheme() {
        assert_eq!(truncate_with_ellipsis("auth cleanup", 20), "auth cleanup");
        assert_eq!(truncate_with_ellipsis("auth cleanup", 8), "auth cl…");
        assert_eq!(truncate_with_ellipsis("漢字漢字", 5), "漢字…");
        assert_eq!(truncate_with_ellipsis("漢字漢字", 4), "漢…");
        assert_eq!(truncate_with_ellipsis("abc", 1), "…");
        assert_eq!(truncate_with_ellipsis("abc", 0), "");
    }

    #[test]
    fn pad_fills_to_the_width_in_cells() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("漢", 4), "漢  ");
        assert_eq!(pad("abcdef", 4), "abcdef");
    }
}
