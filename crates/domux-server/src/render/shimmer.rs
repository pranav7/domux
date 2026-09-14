//! Where the bright band sits along a working agent's word, carried over from V1's
//! `shimmer.go` unchanged, and how lit the compacting arrow is as it breathes.
//!
//! This module answers how lit one character is, and nothing about which colours it is lit
//! between: `theme::Shimmer` holds the two ends of a band and `theme::Breath` the three
//! colours of a breath, and each turns a lit fraction into a colour.
//!
//! The band runs from before the first character to past the last and back. It moves on the
//! same 70 ms tick the glyph counts (`agents::labels::ANIMATION_INTERVAL`), which is why the glyph
//! holds each of its frames for two of them: at one frame a tick the glyph flickers under a
//! band that glides.

/// How far past each end the peak travels, so the band enters and leaves the word rather than
/// appearing on the first character (V1's `tail`). V1 also floors the length of a pass at
/// twelve characters; two tails already come to that, so the floor can never bind and is not
/// carried over.
const TAIL: f64 = 6.0;
/// Characters per tick. Fractional, so the peak slides between two characters across frames
/// instead of jumping a whole one (V1's `speed`).
const SPEED: f64 = 1.2;
/// The width of the band, in characters (V1's `sigma`).
const SIGMA: f64 = 2.2;
/// The least lit any character gets. Without it the word away from the band fades towards the
/// dim end and stops reading as a word (V1's `floor`).
const FLOOR: f64 = 0.35;

/// Which character the band is centred on at `tick`, for a word `len` characters long.
///
/// It counts up to past the far end, then back down, so one pass lights the word in each
/// direction rather than snapping back to the front.
fn peak(len: usize, tick: u64) -> f64 {
    let one_way = len as f64 + TAIL * 2.0;
    let cycle = one_way * 2.0;
    // `tick` counts up from zero and never goes back, so the remainder is never negative and
    // V1's guard for that case has nothing to guard here.
    let phase = (tick as f64 * SPEED) % cycle;
    let phase = if phase > one_way {
        cycle - phase
    } else {
        phase
    };
    phase - TAIL
}

/// How lit character `index` of a word `len` characters long is at `tick`: 0 is the dim end of
/// the band and 1 the bright end. Never below `FLOOR`, and never above 1.
pub fn lit(len: usize, index: usize, tick: u64) -> f64 {
    let d = index as f64 - peak(len, tick);
    (-(d * d) / (2.0 * SIGMA * SIGMA)).exp().max(FLOOR)
}

/// Ticks in one breath of the compacting arrow, dim to bright and back: twenty-eight 70 ms
/// ticks, just under two seconds. Four to a quarter, so a quarter of a breath lands on a
/// tick, and that is where the arrow rests in the `compacting` colour on its way up and down.
pub const BREATH_TICKS: u64 = 28;

/// How lit the compacting arrow is at `tick`: 0 at the dim end and 1 at the bright end
/// (MUX-48).
///
/// A cosine rather than a straight rise and fall, so the colour lingers at each end and
/// hurries through the middle, which is what reads as breathing rather than as a colour
/// sliding back and forth. The arrow is one cell and holds still, so this is the whole of its
/// animation.
pub fn breath(tick: u64) -> f64 {
    let phase = (tick % BREATH_TICKS) as f64 / BREATH_TICKS as f64;
    (1.0 - (phase * std::f64::consts::TAU).cos()) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eleven characters, the length of "Percolating".
    const LEN: usize = 11;
    /// One pass out and back, in ticks: `(LEN + 2 * TAIL) * 2 / SPEED`, rounded up.
    const CYCLE: u64 = 39;

    /// The band walks from before the first character to past the last and turns round once.
    /// Reading the peak rather than the lit characters, because the peak is off the word at
    /// each end of a pass and there is nothing to read there.
    #[test]
    fn the_band_walks_past_both_ends_and_turns_round() {
        let walk: Vec<f64> = (0..CYCLE).map(|t| peak(LEN, t)).collect();
        let top = walk.iter().cloned().fold(f64::MIN, f64::max);
        let bottom = walk.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            bottom < 0.0,
            "it starts before the first character, at {bottom}"
        );
        assert!(
            top > (LEN - 1) as f64,
            "and goes past the last one, to {top}"
        );
        let turns = walk
            .windows(3)
            .filter(|w| w[1] > w[0] && w[1] > w[2])
            .count();
        assert_eq!(turns, 1, "turning once over one pass: {walk:?}");
    }

    /// While the band is on the word, brightness rises to one place and falls away from it.
    /// That is what makes it read as a band rather than as the whole word pulsing.
    #[test]
    fn brightness_rises_to_one_place_and_falls_away() {
        let mut lit_ticks = 0;
        for tick in 0..CYCLE {
            let row: Vec<f64> = (0..LEN).map(|i| lit(LEN, i, tick)).collect();
            if row.iter().all(|v| *v <= FLOOR) {
                continue;
            }
            lit_ticks += 1;
            let places = row
                .windows(3)
                .filter(|w| w[1] > w[0] && w[1] > w[2])
                .count();
            assert!(places <= 1, "one band at tick {tick}: {row:?}");
        }
        assert!(
            lit_ticks >= 25,
            "the band is on the word for most of a pass, {lit_ticks} ticks of {CYCLE}"
        );
    }

    /// No character ever goes dark, so the word stays readable while the band is elsewhere.
    #[test]
    fn no_character_falls_below_the_floor() {
        for tick in 0..200 {
            for i in 0..LEN {
                let v = lit(LEN, i, tick);
                assert!(
                    (FLOOR..=1.0).contains(&v),
                    "character {i} at tick {tick} is lit {v}"
                );
            }
        }
    }

    /// The compacting arrow's breath starts at the dim end, reaches the bright end half way
    /// through and comes back, and one breath later it is where it began (MUX-48).
    #[test]
    fn a_breath_runs_dim_to_bright_and_back_once_per_breath() {
        let quarter = BREATH_TICKS / 4;
        assert_eq!(breath(0), 0.0, "it starts at the dim end");
        assert!(
            (breath(quarter) - 0.5).abs() < 1e-9,
            "half lit a quarter in"
        );
        assert_eq!(breath(quarter * 2), 1.0, "the bright end half way");
        assert!(
            (breath(quarter * 3) - 0.5).abs() < 1e-9,
            "half lit on the way back"
        );
        for tick in 0..BREATH_TICKS * 3 {
            assert_eq!(breath(tick), breath(tick % BREATH_TICKS), "tick {tick}");
        }
        let rise: Vec<f64> = (0..=quarter * 2).map(breath).collect();
        assert!(
            rise.windows(2).all(|w| w[1] > w[0]),
            "brighter on every tick up to the top: {rise:?}"
        );
        let fall: Vec<f64> = (quarter * 2..=BREATH_TICKS).map(breath).collect();
        assert!(
            fall.windows(2).all(|w| w[1] < w[0]),
            "and dimmer on every tick back down: {fall:?}"
        );
    }

    /// A longer word takes longer to cross, so the band moves at one speed whatever it is
    /// running along.
    #[test]
    fn a_longer_word_takes_a_longer_pass() {
        let same_place = |len: usize, tick: u64| (peak(len, tick) * 1000.0).round();
        assert_eq!(
            same_place(4, 5),
            same_place(30, 5),
            "both start at the same speed"
        );
        assert_ne!(
            same_place(4, 30),
            same_place(30, 30),
            "and the shorter one has turned round first"
        );
    }
}
