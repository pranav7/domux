//! The filter's match: which rows a typed filter keeps, and which of them it matches best.
//!
//! A row offers its fields as one string, the fields parted by [`FIELD_SEPARATOR`]. The filter
//! is split on spaces into terms, and a row is kept when every term matches in one of its fields.
//! A term matches a field when its letters appear in the field in order, without case, with any
//! number of letters between them: `todagent` matches `tod-agent-perf`, and so does `tap`.
//!
//! A term never runs from one field into the next, so a letter at the end of a project's name and
//! the next at the start of a branch do not make a match between them. Two terms can match in two
//! fields, which is how `audrey tod` finds the `tod` workspace of the `audrey-app` project.
//!
//! [`score`] also says how well a row matched, so a list can put its fill on the best match
//! without reordering itself. Letters that run together score more than letters spread apart,
//! and a letter at the start of a field or of a word in it scores more than one inside a word.
//! So a name typed whole beats the same letters scattered through a longer one.

/// What parts the fields of a row's filter text. A control character, so no name can hold it.
pub const FIELD_SEPARATOR: char = '\u{1f}';

/// Each letter of a term that matched.
const MATCH: i32 = 16;
/// A letter straight after the one before it.
const RUN: i32 = 16;
/// A letter at the start of a word: after a space, a hyphen or any other mark.
const WORD_START: i32 = 12;
/// A letter at the start of the field, on top of the start of a word.
const FIELD_START: i32 = 8;
/// Each letter skipped between two letters of the term.
const GAP: i32 = 2;

/// How well `query` matches `text`, or `None` when some term of it matches in no field. Higher
/// is better. An empty query matches everything with a score of 0.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query = query.to_lowercase();
    let fields: Vec<Vec<char>> = text
        .to_lowercase()
        .split(FIELD_SEPARATOR)
        .map(|f| f.chars().collect())
        .collect();
    let mut total = 0;
    for term in query.split_whitespace() {
        let term: Vec<char> = term.chars().collect();
        total += fields.iter().filter_map(|f| field_score(&term, f)).max()?;
    }
    Some(total)
}

/// The best score of `term` against one field, over every way its letters can land there.
///
/// `best[j]` is the best score for the letters matched so far with the last of them on `field[j]`.
/// Each letter looks back at the one before it: straight before, for the run, or anywhere further
/// back, less the gap. The gap is linear, so the best "anywhere further back" is a running maximum
/// and each letter costs one pass over the field.
fn field_score(term: &[char], field: &[char]) -> Option<i32> {
    let (first, rest) = term.split_first()?;
    if term.len() > field.len() {
        return None;
    }
    let bonus: Vec<i32> = (0..field.len()).map(|j| position_bonus(field, j)).collect();
    let mut best: Vec<Option<i32>> = field
        .iter()
        .enumerate()
        .map(|(j, c)| (c == first).then(|| MATCH + bonus[j]))
        .collect();
    for letter in rest {
        let mut next = vec![None; field.len()];
        // The best `best[k] + GAP * k` for every `k` at least two behind `j`, so that
        // `reach - GAP * (j - 1)` is that `k`'s score less the letters skipped between them.
        let mut reach: Option<i32> = None;
        for j in 1..field.len() {
            if j >= 2 {
                if let Some(s) = best[j - 2] {
                    let s = s + GAP * (j as i32 - 2);
                    reach = Some(reach.map_or(s, |r| r.max(s)));
                }
            }
            if field[j] != *letter {
                continue;
            }
            let run = best[j - 1].map(|s| s + RUN);
            let skip = reach.map(|r| r - GAP * (j as i32 - 1));
            next[j] = run.max(skip).map(|s| s + MATCH + bonus[j]);
        }
        best = next;
    }
    best.into_iter().flatten().max()
}

fn position_bonus(field: &[char], j: usize) -> i32 {
    match j {
        0 => WORD_START + FIELD_START,
        _ if !field[j - 1].is_alphanumeric() && field[j].is_alphanumeric() => WORD_START,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(parts: &[&str]) -> String {
        parts.join(&FIELD_SEPARATOR.to_string())
    }

    #[test]
    fn score_empty_query_matches_everything() {
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("   ", ""), Some(0));
    }

    #[test]
    fn score_letters_in_order_match_with_letters_between() {
        assert!(score("todagent", "tod-agent-perf").is_some());
        assert!(score("tap", "tod-agent-perf").is_some());
        assert!(score("tdgntprf", "tod-agent-perf").is_some());
    }

    #[test]
    fn score_letters_out_of_order_do_not_match() {
        assert_eq!(score("pat", "tod-agent-perf"), None);
        assert_eq!(score("todd", "tod-agent-perf"), None);
    }

    #[test]
    fn score_ignores_case() {
        assert!(score("TodAgent", "tod-agent-perf").is_some());
        assert!(score("audrey", "AUDREY-APP").is_some());
    }

    #[test]
    fn score_term_does_not_run_across_fields() {
        let text = fields(&["audrey-app", "tod-agent-perf"]);
        assert_eq!(score("apptod", &text), None);
        assert!(score("app", &text).is_some());
        assert!(score("tod", &text).is_some());
    }

    #[test]
    fn score_every_term_must_match_in_some_field() {
        let text = fields(&["audrey-app", "tod-agent-perf", "main"]);
        assert!(score("audrey tod", &text).is_some());
        assert!(score("tod audrey", &text).is_some());
        assert_eq!(score("audrey billing", &text), None);
    }

    #[test]
    fn score_run_beats_the_same_letters_spread_out() {
        let run = score("tod", "tod-agent-perf").unwrap();
        let spread = score("tod", "the-only-draft").unwrap();
        assert!(run > spread, "{run} should beat {spread}");
    }

    #[test]
    fn score_start_of_field_beats_middle_of_word() {
        let start = score("agent", "agent-perf").unwrap();
        let middle = score("agent", "reagent-perf").unwrap();
        assert!(start > middle, "{start} should beat {middle}");
    }

    #[test]
    fn score_start_of_word_beats_middle_of_word() {
        let word = score("perf", "tod-perf").unwrap();
        let inside = score("perf", "todperf").unwrap();
        assert!(word > inside, "{word} should beat {inside}");
    }

    #[test]
    fn score_fewer_letters_skipped_beats_more() {
        let near = score("ta", "t-a").unwrap();
        let far = score("ta", "t---a").unwrap();
        assert!(near > far, "{near} should beat {far}");
    }

    #[test]
    fn score_finds_the_best_landing_not_the_first() {
        // A greedy match takes the `a` of `tab` and scores the spread. The run is later in the
        // field and must still win.
        let later = score("agent", "tab-agent").unwrap();
        let spread = score("agent", "tab-a-g-e-n-t").unwrap();
        assert!(later > spread, "{later} should beat {spread}");
    }

    #[test]
    fn score_term_longer_than_every_field_does_not_match() {
        assert_eq!(score("abcdef", &fields(&["abc", "def"])), None);
    }

    #[test]
    fn score_matches_letters_outside_ascii() {
        assert!(score("été", "l-été-dernier").is_some());
        assert!(score("ÉTÉ", "l-été-dernier").is_some());
    }
}
