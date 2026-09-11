//! The working word and the animated glyph, carried over from V1 unchanged.
//!
//! The list is V1's `aiWorkingLabels` (`ai_working_labels.go`); the frames and the 80 ms
//! interval are V1's `picker.go` `claudeSpinnerFrames` and `pickerSpinnerInterval`. The word
//! is picked per agent, stays the same while the agent works, and is never the same word
//! twice on screen. The words carry no ellipsis; the row appends `…` when it draws them.

use domux_core::ids::AgentId;
use std::collections::HashMap;
use std::time::Duration;

pub const WORDS: [&str; 186] = [
    "Accomplishing",
    "Actioning",
    "Actualizing",
    "Architecting",
    "Baking",
    "Beaming",
    "Beboppin'",
    "Befuddling",
    "Billowing",
    "Blanching",
    "Bloviating",
    "Boogieing",
    "Boondoggling",
    "Booping",
    "Bootstrapping",
    "Brewing",
    "Bunning",
    "Burrowing",
    "Calculating",
    "Canoodling",
    "Caramelizing",
    "Cascading",
    "Catapulting",
    "Cerebrating",
    "Channeling",
    "Channelling",
    "Choreographing",
    "Churning",
    "Coalescing",
    "Cogitating",
    "Combobulating",
    "Composing",
    "Computing",
    "Concocting",
    "Considering",
    "Contemplating",
    "Cooking",
    "Crafting",
    "Creating",
    "Crunching",
    "Crystallizing",
    "Cultivating",
    "Deciphering",
    "Deliberating",
    "Determining",
    "Dilly-dallying",
    "Discombobulating",
    "Doing",
    "Doodling",
    "Drizzling",
    "Ebbing",
    "Effecting",
    "Elucidating",
    "Embellishing",
    "Enchanting",
    "Envisioning",
    "Evaporating",
    "Fermenting",
    "Fiddle-faddling",
    "Finagling",
    "Flambéing",
    "Flibbertigibbeting",
    "Flowing",
    "Flummoxing",
    "Fluttering",
    "Forging",
    "Forming",
    "Frolicking",
    "Frosting",
    "Gallivanting",
    "Galloping",
    "Garnishing",
    "Generating",
    "Gesticulating",
    "Germinating",
    "Gitifying",
    "Grooving",
    "Gusting",
    "Harmonizing",
    "Hashing",
    "Hatching",
    "Herding",
    "Honking",
    "Hullaballooing",
    "Hyperspacing",
    "Ideating",
    "Imagining",
    "Improvising",
    "Incubating",
    "Inferring",
    "Infusing",
    "Ionizing",
    "Jitterbugging",
    "Julienning",
    "Kneading",
    "Leavening",
    "Levitating",
    "Lollygagging",
    "Manifesting",
    "Marinating",
    "Meandering",
    "Metamorphosing",
    "Misting",
    "Moonwalking",
    "Moseying",
    "Mulling",
    "Mustering",
    "Musing",
    "Nebulizing",
    "Nesting",
    "Newspapering",
    "Noodling",
    "Nucleating",
    "Orbiting",
    "Orchestrating",
    "Osmosing",
    "Perambulating",
    "Percolating",
    "Perusing",
    "Philosophising",
    "Photosynthesizing",
    "Pollinating",
    "Pondering",
    "Pontificating",
    "Pouncing",
    "Precipitating",
    "Prestidigitating",
    "Processing",
    "Proofing",
    "Propagating",
    "Puttering",
    "Puzzling",
    "Quantumizing",
    "Razzle-dazzling",
    "Razzmatazzing",
    "Recombobulating",
    "Reticulating",
    "Roosting",
    "Ruminating",
    "Sautéing",
    "Scampering",
    "Schlepping",
    "Scurrying",
    "Seasoning",
    "Shenaniganing",
    "Shimmying",
    "Simmering",
    "Skedaddling",
    "Sketching",
    "Slithering",
    "Smooshing",
    "Sock-hopping",
    "Spelunking",
    "Spinning",
    "Sprouting",
    "Stewing",
    "Sublimating",
    "Swirling",
    "Swooping",
    "Symbioting",
    "Synthesizing",
    "Tempering",
    "Thinking",
    "Thundering",
    "Tinkering",
    "Tomfoolering",
    "Topsy-turvying",
    "Transfiguring",
    "Transmuting",
    "Twisting",
    "Undulating",
    "Unfurling",
    "Unravelling",
    "Vibing",
    "Waddling",
    "Wandering",
    "Warping",
    "Whatchamacalliting",
    "Whirlpooling",
    "Whirring",
    "Whisking",
    "Wibbling",
    "Working",
    "Wrangling",
    "Zesting",
    "Zigzagging",
];

/// Sparse to dense and back, so each frame morphs into the next (V1's comment).
pub const GLYPH_FRAMES: [&str; 13] = [
    "·", "✦", "✶", "✳", "✢", "✻", "✽", "✻", "✢", "✳", "✶", "✦", "·",
];

/// One animation tick every 80 ms (V1's `pickerSpinnerInterval`). The band along a working
/// word moves on every one of them and the glyph turns on every second one.
///
/// It has a floor that is not V1's. While anything works the core pushes a frame every
/// interval, and `testing::Harness::pump` returns only after 50 ms with no message, so an
/// interval at or below that window would leave every `frame()` call during work reading
/// frames until the test timed out. Lowering this number means raising that one.
pub const ANIMATION_INTERVAL: Duration = Duration::from_millis(80);

/// Ticks per frame of the glyph, so it turns every 160 ms while the band along the working
/// word moves every 80 ms (V1's `renderAIBadges`, "icon advances every 2 ticks").
///
/// Two animations on one counter rather than two tickers. The band has to move on every tick
/// to glide, and a glyph that turned that fast flickered under it (MUX-26).
pub const GLYPH_TICKS_PER_FRAME: u64 = 2;

/// The frame for an animation tick counter. The counter is the core's, so every client on
/// the same server draws the same frame.
pub fn frame_at(tick: u64) -> &'static str {
    let frame = tick / GLYPH_TICKS_PER_FRAME;
    GLYPH_FRAMES[(frame % GLYPH_FRAMES.len() as u64) as usize]
}

/// Which word each working agent has. The core owns one of these.
#[derive(Default)]
pub struct WorkingWords {
    assigned: HashMap<AgentId, &'static str>,
}

impl WorkingWords {
    /// The word for `agent`, picking one on the first call. V1's rule: FNV-1a over the id
    /// gives the starting index, then walk forward past words another agent already has.
    pub fn word_for(&mut self, agent: &AgentId) -> &'static str {
        if let Some(w) = self.assigned.get(agent) {
            return w;
        }
        let start = fnv1a(agent.as_str()) as usize % WORDS.len();
        let taken: std::collections::HashSet<&str> = self.assigned.values().copied().collect();
        let mut chosen = WORDS[start];
        for i in 0..WORDS.len() {
            let candidate = WORDS[(start + i) % WORDS.len()];
            if !taken.contains(candidate) {
                chosen = candidate;
                break;
            }
        }
        self.assigned.insert(agent.clone(), chosen);
        chosen
    }

    /// The agent stopped working or went away; its word is free again.
    pub fn release(&mut self, agent: &AgentId) {
        self.assigned.remove(agent);
    }

    pub fn in_use(&self) -> usize {
        self.assigned.len()
    }
}

/// FNV-1a, 32 bit, the hash V1's `stableAIWorkingLabelExcept` uses.
fn fnv1a(s: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for b in s.as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::ids::AgentId;
    use std::collections::HashSet;

    #[test]
    fn the_word_list_and_the_frames_match_v1() {
        assert_eq!(WORDS.len(), 186);
        assert_eq!(WORDS[0], "Accomplishing");
        assert_eq!(WORDS[WORDS.len() - 1], "Zigzagging");
        assert!(WORDS.contains(&"Percolating"));
        assert!(WORDS.contains(&"Kneading"));
        assert!(WORDS.contains(&"Marinating"));
        assert!(
            WORDS.iter().all(|w| !w.ends_with('…')),
            "the ellipsis is added when drawing"
        );
        assert_eq!(
            GLYPH_FRAMES,
            [
                "·", "✦", "✶", "✳", "✢", "✻", "✽", "✻", "✢", "✳", "✶", "✦", "·"
            ]
        );
        assert_eq!(ANIMATION_INTERVAL, std::time::Duration::from_millis(80));
    }

    #[test]
    fn a_word_is_stable_for_one_agent_across_calls() {
        let mut w = WorkingWords::default();
        let a = AgentId("a_5e21".into());
        let first = w.word_for(&a);
        for _ in 0..50 {
            assert_eq!(w.word_for(&a), first);
        }
    }

    #[test]
    fn no_two_agents_on_screen_share_a_word() {
        let mut w = WorkingWords::default();
        let ids: Vec<AgentId> = (0..40).map(|i| AgentId(format!("a_{i:04x}"))).collect();
        let words: Vec<&str> = ids.iter().map(|id| w.word_for(id)).collect();
        assert_eq!(words.iter().collect::<HashSet<_>>().len(), 40);
        assert_eq!(w.in_use(), 40);
    }

    #[test]
    fn releasing_an_agent_frees_its_word_for_the_next_one() {
        let mut w = WorkingWords::default();
        let a = AgentId("a_0001".into());
        let taken = w.word_for(&a);
        w.release(&a);
        assert_eq!(w.in_use(), 0);
        let b = AgentId("a_0001".into());
        assert_eq!(
            w.word_for(&b),
            taken,
            "the same id picks the same word again"
        );
    }

    #[test]
    fn the_glyph_cycles_through_thirteen_frames_in_order() {
        assert_eq!(frame_at(0), "·");
        assert_eq!(frame_at(4), "✶");
        assert_eq!(frame_at(26), "·");
        assert_eq!(frame_at(30), "✶");
    }

    /// Each frame is held for two ticks, so the glyph turns at half the rate the band does.
    #[test]
    fn the_glyph_holds_each_frame_for_two_ticks() {
        for tick in 0..GLYPH_FRAMES.len() as u64 * GLYPH_TICKS_PER_FRAME * 2 {
            assert_eq!(
                frame_at(tick),
                frame_at(tick - tick % GLYPH_TICKS_PER_FRAME),
                "tick {tick} shows the frame its pair started on"
            );
        }
        assert_ne!(
            frame_at(0),
            frame_at(GLYPH_TICKS_PER_FRAME),
            "and the next pair shows the next frame"
        );
    }
}
