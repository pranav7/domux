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

/// Strips what a terminal cannot draw as a cell: control characters, and graphemes that
/// occupy no cells at all. Program-controlled text reaches the UI verbatim - a pane's title
/// is whatever it emitted with OSC 0 or OSC 2 - so every such string passes through here
/// before it is rendered.
///
/// Two concrete failures this prevents. A control character reports a display width of 1, so
/// it satisfies every budget and is then written into a cell: ratatui flushes the byte and a
/// literal newline inside a border desynchronises the whole screen. A zero-width grapheme
/// reports 0, so a title of them satisfies any budget while still consuming one cell each
/// when drawn, and the row runs past its box.
///
/// After this, every remaining grapheme occupies at least one cell, so a width measured here
/// is the width that will be drawn.
pub fn sanitize_for_display(s: &str) -> String {
    s.graphemes(true)
        .filter(|g| !g.chars().any(char::is_control) && grapheme_width(g) > 0)
        .collect()
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

/// Splits `s` into lines of at most `width` cells, breaking between words.
///
/// Measured in cells through the same `display_width` every other row uses, so a wrapped
/// notice occupies the cells it says it does. Breaking is on whitespace, and runs of
/// whitespace collapse: a wrapped line never starts with a space it inherited from the break.
///
/// A word wider than the whole line is broken across lines by grapheme rather than dropped
/// or left to run past the edge, so no future rewording can make this drop text or hang. A
/// single grapheme wider than the line - a wide glyph on a one-column screen - goes on a
/// line of its own and overflows it; the renderer clips, which is the only honest thing left
/// at that size.
///
/// A zero `width` has no line to put anything on, so the answer is no lines.
pub fn wrap_to_width(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut used = 0usize;
    for word in s.split_whitespace() {
        let w = display_width(word);
        if used > 0 && used + 1 + w > width {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if w > width {
            for g in word.graphemes(true) {
                // At least one cell per grapheme, so a zero-width one cannot make the
                // accounting stand still while the line grows.
                let gw = grapheme_width(g).max(1);
                if used > 0 && used + gw > width {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push_str(g);
                used += gw;
            }
            continue;
        }
        if used > 0 {
            line.push(' ');
            used += 1;
        }
        line.push_str(word);
        used += w;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
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
    fn wrap_to_width_breaks_between_words_and_measures_in_cells() {
        assert_eq!(
            wrap_to_width("Screen is 30x8. domux needs at least 40x10.", 30),
            vec!["Screen is 30x8. domux needs at", "least 40x10."]
        );
        // Exactly the width fits; one cell more does not.
        assert_eq!(wrap_to_width("ab cd", 5), vec!["ab cd"]);
        assert_eq!(wrap_to_width("ab cde", 5), vec!["ab", "cde"]);
        // Cells, not chars: two wide glyphs fill a four-cell line.
        assert_eq!(wrap_to_width("漢字 x", 4), vec!["漢字", "x"]);
        assert_eq!(wrap_to_width("", 10), Vec::<String>::new());
        assert_eq!(wrap_to_width("anything", 0), Vec::<String>::new());
    }

    /// The wording of a notice is a thing people edit. A word longer than the screen must
    /// break rather than be dropped, run past the edge, or spin - none of which a reader
    /// changing the words would be warned about.
    #[test]
    fn wrap_to_width_breaks_a_word_that_is_wider_than_the_line() {
        assert_eq!(
            wrap_to_width("a supercalifragilistic word", 8),
            vec!["a", "supercal", "ifragili", "stic", "word"]
        );
        // One cell wide: every grapheme lands on its own line and nothing is lost.
        assert_eq!(wrap_to_width("abc", 1), vec!["a", "b", "c"]);
        // A grapheme wider than the whole line still gets a line; the renderer clips it.
        assert_eq!(wrap_to_width("漢", 1), vec!["漢"]);
    }

    #[test]
    fn sanitize_for_display_drops_control_characters() {
        // `display_width("\n") == 1`, so a control character passes every budget check and is
        // then written into a cell verbatim. Fifteen of them inside a border move the cursor
        // fifteen lines and desynchronise the frame.
        assert_eq!(sanitize_for_display("a\nb"), "ab");
        assert_eq!(sanitize_for_display("a\tb\rc"), "abc");
        assert_eq!(sanitize_for_display("a\u{1b}[31mb"), "a[31mb");
        assert_eq!(sanitize_for_display("bell\u{7}"), "bell");
    }

    #[test]
    fn sanitize_for_display_drops_zero_width_graphemes() {
        // These measure 0 cells but `put` still gives each one a cell, so a title made of
        // them satisfies any budget and then overruns its box.
        assert_eq!(sanitize_for_display("\u{200b}".repeat(40).as_str()), "");
        assert_eq!(sanitize_for_display("a\u{feff}b"), "ab");
        assert_eq!(sanitize_for_display("soft\u{ad}hyphen"), "softhyphen");
    }

    #[test]
    fn sanitize_for_display_keeps_every_drawable_grapheme() {
        assert_eq!(sanitize_for_display("auth cleanup"), "auth cleanup");
        assert_eq!(sanitize_for_display("漢字"), "漢字");
        assert_eq!(sanitize_for_display("é"), "é"); // e plus a combining acute is one cluster
        assert_eq!(sanitize_for_display("👍🏽"), "👍🏽");
    }

    #[test]
    fn sanitized_text_measures_the_width_it_will_draw() {
        // The invariant the renderer depends on: after sanitizing, no grapheme measures 0,
        // so a budget computed from `display_width` is the number of cells `put` will use.
        for raw in ["\u{200b}\u{200b}ab", "a\nb", "漢\u{feff}字", "plain"] {
            let clean = sanitize_for_display(raw);
            let drawn: usize = clean
                .graphemes(true)
                .map(|g| grapheme_width(g).max(1))
                .sum();
            assert_eq!(display_width(&clean), drawn, "raw: {raw:?}");
        }
    }

    #[test]
    fn pad_fills_to_the_width_in_cells() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("漢", 4), "漢  ");
        assert_eq!(pad("abcdef", 4), "abcdef");
    }
}
