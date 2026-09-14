//! The readability guards: what keeps a colour nobody chose for this terminal readable on it.
//!
//! They run after a theme is resolved against the terminal's answers, and only when some role's
//! value was read from them. A role moves toward the far end, white on a dark overlay and black on
//! a light one, in steps of 1/18, until it meets its floor on every ground it is drawn on. Moving
//! toward the far end keeps its hue.
//!
//! A role can sit on grounds on both sides of it, a dark one a theme file wrote and a light one
//! the terminal answered. When no step toward the far end meets the floor on all of them, the
//! least step toward the other end that does is taken; when neither end has one, the step that
//! reads best on the worst of its grounds, no step included. A move never lowers the lowest
//! contrast a role has.

use super::color::{contrast, luminance, mix, oklch};
use super::role::{Guard, Role};
use super::value::ColorValue;
use super::Paint;
use domux_term::Rgb;

/// The contrast a text tier, a colour, a kind or a band end needs on each of its grounds: WCAG 2's
/// minimum for large text and interface marks.
pub const FLOOR: f64 = 3.0;
/// The contrast the rule needs on each of its grounds. It is lower than `LINE_FLOOR` because the
/// rule on a Mocha or a Ristretto terminal is under 1.4 on the overlay background, and neither
/// should move.
pub const RULE_FLOOR: f64 = 1.25;
/// The contrast the separator and the border need on each of their grounds.
pub const LINE_FLOOR: f64 = 1.4;
/// The contrast the stay awake dot for "not held" needs on each of its grounds.
pub const DOT_OFF_FLOOR: f64 = 2.0;
/// The contrast the top bar, the toast and the selected row keep from the overlay background.
pub const GROUND_FLOOR: f64 = 1.05;
/// A move is in steps of 1/STEPS of the way.
pub const STEPS: u8 = 18;
/// The OKLCH chroma under which a colour has no hue to test.
pub const MIN_CHROMA: f64 = 0.03;
/// A red's OKLCH hue is at least this, or at most `RED_TO`.
pub const RED_FROM: f64 = 345.0;
/// A red's OKLCH hue is at most this, or at least `RED_FROM`.
pub const RED_TO: f64 = 40.0;
/// A green's OKLCH hue is at least this.
pub const GREEN_FROM: f64 = 115.0;
/// A green's OKLCH hue is at most this.
pub const GREEN_TO: f64 = 170.0;

/// True when a colour can stand for a role of this guard: red for `Red`, green for `Green`, and
/// any colour for the rest.
pub fn passes_hue(guard: Guard, c: Rgb) -> bool {
    let in_window = |hue: f64| match guard {
        Guard::Red => hue >= RED_FROM || hue <= RED_TO,
        _ => (GREEN_FROM..=GREEN_TO).contains(&hue),
    };
    match guard {
        Guard::Red | Guard::Green => {
            let (_, chroma, hue) = oklch(c);
            chroma >= MIN_CHROMA && in_window(hue)
        }
        Guard::Ground
        | Guard::TextTier
        | Guard::Colour
        | Guard::Kind
        | Guard::Line
        | Guard::Unguarded => true,
    }
}

/// One role's value after resolution: what it paints, the value that painted it, and the index
/// of the layer that wrote it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Resolved {
    pub paint: Paint,
    pub value: ColorValue,
    pub layer: usize,
}

impl Resolved {
    /// True when the value was read from the terminal's answers. A `default` ground is not.
    pub fn read_answers(&self) -> bool {
        !matches!(self.value, ColorValue::Hex(_) | ColorValue::Default)
    }
}

/// What the guards did, for the tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Trace {
    /// The steps each role moved.
    pub steps: [u8; Role::COUNT],
    /// The roles whose palette slot left its hue when moved, so the next layer's value was used.
    pub hue_again: Vec<Role>,
}

impl Default for Trace {
    fn default() -> Trace {
        Trace {
            steps: [0; Role::COUNT],
            hue_again: Vec::new(),
        }
    }
}

/// Runs the three passes over a resolved theme. `bg` is the answered background, which a
/// `default` ground stands for; `root` is the index of the `domux` layer; `next(role, layer)`
/// resolves a role again from that layer on.
pub(super) fn run(
    resolved: &mut [Resolved; Role::COUNT],
    bg: Rgb,
    root: usize,
    next: impl Fn(Role, usize) -> Resolved,
) -> Trace {
    let mut trace = Trace::default();
    let colour =
        |resolved: &[Resolved; Role::COUNT], role: Role| match resolved[role as usize].paint {
            Paint::Rgb(c) => c,
            Paint::Default => bg,
        };
    let text = colour(resolved, Role::Text);
    let far = if luminance(colour(resolved, Role::OverlayBackground)) < luminance(text) {
        WHITE
    } else {
        BLACK
    };
    // A role may move when its own value came from the answers, or when it is the root's hex
    // and one of its grounds came from the answers. Any other layer's hex is drawn as written.
    let movable = |resolved: &[Resolved; Role::COUNT], role: Role| {
        let found = resolved[role as usize];
        found.read_answers()
            || (matches!(found.value, ColorValue::Hex(_))
                && found.layer == root
                && role
                    .grounds()
                    .iter()
                    .any(|g| resolved[*g as usize].read_answers()))
    };

    // 1. The top bar, the toast and the selected row keep a step from the overlay background,
    //    moving toward the text.
    for role in Role::ALL.iter().copied() {
        if role.guard() != Guard::Ground
            || role.grounds().is_empty()
            || !resolved[role as usize].read_answers()
        {
            continue;
        }
        let from = colour(resolved, role);
        let on: Vec<Rgb> = role
            .grounds()
            .iter()
            .map(|g| colour(resolved, *g))
            .collect();
        let (to, k) = best_move(&[(from, on)], &[text], GROUND_FLOOR);
        resolved[role as usize].paint = Paint::Rgb(mix(from, to, k, STEPS));
        trace.steps[role as usize] = k;
    }

    // 2. The text tiers written as a blend move together, so they keep their order. A tier
    //    written any other way moves on its own.
    let tiers: Vec<Role> = Role::ALL
        .iter()
        .copied()
        .filter(|r| r.guard() == Guard::TextTier)
        .collect();
    let on = |resolved: &[Resolved; Role::COUNT], role: Role| -> Vec<Rgb> {
        role.grounds()
            .iter()
            .map(|g| colour(resolved, *g))
            .collect()
    };
    let blends: Vec<Role> = tiers
        .iter()
        .copied()
        .filter(|r| matches!(resolved[*r as usize].value, ColorValue::Blend { .. }))
        .collect();
    let together: Vec<(Rgb, Vec<Rgb>)> = blends
        .iter()
        .map(|r| (colour(resolved, *r), on(resolved, *r)))
        .collect();
    let ends = [far, if far == WHITE { BLACK } else { WHITE }];
    let (to, k) = best_move(&together, &ends, FLOOR);
    for (role, (from, _)) in blends.iter().zip(&together) {
        resolved[*role as usize].paint = Paint::Rgb(mix(*from, to, k, STEPS));
        trace.steps[*role as usize] = k;
    }
    for role in tiers {
        if blends.contains(&role) || !movable(resolved, role) {
            continue;
        }
        let from = colour(resolved, role);
        let (to, k) = best_move(&[(from, on(resolved, role))], &ends, FLOOR);
        resolved[role as usize].paint = Paint::Rgb(mix(from, to, k, STEPS));
        trace.steps[role as usize] = k;
    }

    // 3. Each colour, kind, band end and line moves on its own, to its own floor. A red or green
    //    palette slot that leaves its hue on the way takes the next layer's value, which is
    //    guarded in turn.
    for role in Role::ALL.iter().copied() {
        let guard = role.guard();
        let floor = match (guard, role.floor()) {
            (Guard::Colour | Guard::Red | Guard::Green | Guard::Kind | Guard::Line, Some(f)) => f,
            _ => continue,
        };
        while movable(resolved, role) {
            let from = colour(resolved, role);
            let (to, k) = best_move(&[(from, on(resolved, role))], &ends, floor);
            let moved = mix(from, to, k, STEPS);
            let found = resolved[role as usize];
            if matches!(found.value, ColorValue::Palette(_)) && !passes_hue(guard, moved) {
                trace.hue_again.push(role);
                resolved[role as usize] = next(role, found.layer + 1);
                continue;
            }
            resolved[role as usize].paint = Paint::Rgb(moved);
            trace.steps[role as usize] = k;
            break;
        }
    }
    trace
}

const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
const WHITE: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};

/// Where a set of colours moves together: toward which of `ends`, and by how many steps from 0
/// to `STEPS`.
///
/// A move is allowed only when no colour reads worse on its grounds than it did unmoved, so a
/// move never lowers any colour's lowest contrast, and not moving is always allowed. An allowed
/// move is scored by the lowest contrast any colour has on any of its grounds, counted as no
/// more than `floor`, and the first best move wins, trying the ends in order and the steps from
/// 0. So it is the least k toward the first end that meets the floor everywhere, then the least
/// toward the next end; when no move meets it, the move that reads best on the worst ground.
fn best_move(colours: &[(Rgb, Vec<Rgb>)], ends: &[Rgb], floor: f64) -> (Rgb, u8) {
    // Each colour's lowest contrast on its grounds after k steps toward `to`.
    let lowest = |to: Rgb, k: u8| -> Vec<f64> {
        colours
            .iter()
            .map(|(from, on)| {
                let moved = mix(*from, to, k, STEPS);
                on.iter()
                    .map(|ground| contrast(moved, *ground))
                    .fold(f64::INFINITY, f64::min)
            })
            .collect()
    };
    let unmoved = lowest(ends[0], 0);
    let score = |each: &[f64]| each.iter().copied().fold(floor, f64::min);
    let mut best = (ends[0], 0, score(&unmoved));
    for to in ends.iter().copied() {
        for k in 1..=STEPS {
            if best.2 >= floor {
                return (best.0, best.1);
            }
            let each = lowest(to, k);
            let allowed = each.iter().zip(&unmoved).all(|(now, was)| now >= was);
            if allowed && score(&each) > best.2 {
                best = (to, k, score(&each));
            }
        }
    }
    (best.0, best.1)
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{self, OMARCHY};
    use super::super::{builtin, resolve, Chain, TerminalColors, Theme, ThemeChoice};
    use super::*;

    fn rgb(v: u32) -> Rgb {
        Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    fn hex(v: u32) -> Paint {
        Paint::Rgb(rgb(v))
    }

    fn terminal() -> &'static Chain {
        builtin::chain("terminal").expect("built in")
    }

    fn answers(name: &str) -> TerminalColors {
        fixture::theme(name).colors()
    }

    /// The built-in terminal theme painted on an Omarchy theme's answers.
    fn on(name: &str) -> Theme {
        Theme::paint(terminal(), &answers(name))
    }

    /// A chain of one file over what it extends.
    fn chain_of(text: &str) -> Chain {
        let (chain, warnings) = resolve(
            &ThemeChoice::File("mine".to_string()),
            builtin::BUILTIN,
            |_| Ok(Some(text.to_string())),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        chain.expect("resolves")
    }

    /// A role's colour, with a `default` ground standing for the answered background.
    fn colour(theme: &Theme, role: Role, colors: &TerminalColors) -> Rgb {
        match theme.get(role) {
            Paint::Rgb(c) => c,
            Paint::Default => colors
                .bg
                .expect("a default ground on an answered background"),
        }
    }

    /// The lowest contrast a role has on the grounds in its list.
    fn lowest(theme: &Theme, role: Role, colors: &TerminalColors) -> f64 {
        role.grounds()
            .iter()
            .map(|g| contrast(colour(theme, role, colors), colour(theme, *g, colors)))
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn a_yellow_or_amber_slot_is_not_green_and_a_magenta_slot_is_not_red() {
        assert!(!passes_hue(Guard::Green, rgb(0xffff00)));
        assert!(!passes_hue(Guard::Green, rgb(0xffc107)));
        assert!(!passes_hue(Guard::Red, rgb(0xff00ff)));
        assert!(!passes_hue(Guard::Red, rgb(0xd27fb4)));
        assert!(passes_hue(Guard::Green, rgb(0xa6e3a1)));
        assert!(passes_hue(Guard::Red, rgb(0xf38ba8)));
        assert!(passes_hue(Guard::Red, rgb(0xff0000)));
        // A grey has no hue, whatever its angle.
        assert!(!passes_hue(Guard::Red, rgb(0x3a2e2e)));
        // The other guards take any colour.
        assert!(passes_hue(Guard::Colour, rgb(0xffff00)));
        assert!(passes_hue(Guard::TextTier, rgb(0x808080)));

        // Painted: the slot is refused and the role takes the domux value; a role that is not red
        // or green uses the same slots.
        let mut colors = answers("ristretto");
        colors.palette[1] = Some(rgb(0xff00ff));
        colors.palette[2] = Some(rgb(0xffc107));
        colors.palette[4] = Some(rgb(0xffff00));
        let theme = Theme::paint(terminal(), &colors);
        assert_eq!(theme.get(Role::PrClosed), hex(0xf38ba8));
        assert_eq!(theme.get(Role::WaitingDot), hex(0xf38ba8));
        assert_eq!(theme.get(Role::PrOpen), hex(0xa6e3a1));
        assert_eq!(theme.get(Role::StayAwakeDotOn), hex(0xa6e3a1));
        assert_eq!(theme.get(Role::Accent), hex(0xffff00));

        // A hex value is never hue tested.
        let chain = chain_of("extends = \"terminal\"\n[roles]\npr_open = \"#ffff00\"\n");
        assert_eq!(
            Theme::paint(&chain, &colors).get(Role::PrOpen),
            hex(0xffff00)
        );
    }

    #[test]
    fn the_terminal_theme_on_a_mocha_terminal_gives_every_domux_ground_line_and_text_tier() {
        let theme = on("catppuccin");
        let domux = Theme::domux();
        for role in [
            Role::OverlayBackground,
            Role::TopBarBackground,
            Role::ToastBackground,
            Role::Fill,
            Role::SidebarBackground,
            Role::TabRowBackground,
            Role::Rule,
            Role::Separator,
            Role::Border,
            Role::Text,
            Role::SoftText,
            Role::DimText,
            Role::FaintText,
            Role::RecapSeen,
            Role::StayAwakeDotOff,
        ] {
            assert_eq!(theme.get(role), domux.get(role), "{}", role.name());
        }
        assert_eq!(theme.get(Role::TopBarBackground), hex(0x181825));
    }

    #[test]
    fn the_terminal_theme_on_ristretto_moves_nothing() {
        use Role::*;
        let want = [
            (OverlayBackground, hex(0x2c2525)),
            (TopBarBackground, hex(0x231e1e)),
            (ToastBackground, hex(0x231e1e)),
            (Fill, hex(0x413939)),
            (SidebarBackground, Paint::Default),
            (TabRowBackground, Paint::Default),
            (Rule, hex(0x413939)),
            (Separator, hex(0x554d4d)),
            (Border, hex(0x6a6162)),
            (Text, hex(0xe6d9db)),
            (SoftText, hex(0xbdb1b3)),
            (DimText, hex(0x93898a)),
            (FaintText, hex(0x7f7576)),
            (OnAccent, hex(0x2c2525)),
            (OnPill, hex(0x2c2525)),
            (Accent, hex(0xf38d70)),
            (HintKey, hex(0x85dacc)),
            (WorkspaceName, hex(0x85dacc)),
            (Branch, hex(0xa8a9eb)),
            (PrOpen, hex(0xadda78)),
            (PrMerged, hex(0xf38d70)),
            (PrClosed, hex(0xfd6883)),
            (PillOk, hex(0xadda78)),
            (PillError, hex(0xfd6883)),
            (ConfigError, hex(0xfd6883)),
            (Question, hex(0xfd6883)),
            (WaitingDot, hex(0xfd6883)),
            (StayAwakeDotOn, hex(0xadda78)),
            (StayAwakeDotOff, hex(0x6a6162)),
            (Recap, hex(0xe6d9db)),
            (RecapSeen, hex(0xbdb1b3)),
            (Claude, hex(0xde7356)),
            (Codex, hex(0x89b4fa)),
            (Opencode, hex(0xc678b8)),
            (Compacting, hex(0xafafff)),
            (BandClaudeDim, hex(0xb85e47)),
            (BandClaudeBright, hex(0xffc9b0)),
            (BandCodexDim, hex(0x6478a8)),
            (BandCodexBright, hex(0xc8daff)),
            (BandOpencodeDim, hex(0x9f5d93)),
            (BandOpencodeBright, hex(0xf0b5e3)),
            (BandCompactingDim, hex(0x6f6fcf)),
            (BandCompactingBright, hex(0xd8d8ff)),
        ];
        assert_eq!(want.len(), Role::COUNT);
        let (theme, trace) = Theme::paint_traced(terminal(), &answers("ristretto"));
        for (role, paint) in want {
            assert_eq!(theme.get(role), paint, "{}", role.name());
        }
        assert_eq!(trace, Trace::default());
    }

    #[test]
    fn the_terminal_theme_on_catppuccin_latte_moves_the_text_tiers_together_toward_black() {
        let (theme, trace) = Theme::paint_traced(terminal(), &answers("catppuccin-latte"));
        assert_eq!(theme.get(Role::Text), hex(0x4c4f69));
        assert_eq!(theme.get(Role::Recap), hex(0x4c4f69));
        assert_eq!(theme.get(Role::SoftText), hex(0x57596a));
        assert_eq!(theme.get(Role::RecapSeen), hex(0x57596a));
        assert_eq!(theme.get(Role::DimText), hex(0x737582));
        assert_eq!(theme.get(Role::FaintText), hex(0x82838e));
        assert_eq!(theme.get(Role::Branch), hex(0xc362a9));
        for role in [
            Role::SoftText,
            Role::DimText,
            Role::FaintText,
            Role::RecapSeen,
        ] {
            assert_eq!(trace.steps[role as usize], 4, "{}", role.name());
        }
        assert_eq!(trace.steps[Role::Text as usize], 0);
        assert_eq!(trace.steps[Role::Branch as usize], 3);
        // Toward black, in order.
        let lum = |role| luminance(colour(&theme, role, &answers("catppuccin-latte")));
        assert!(lum(Role::FaintText) > lum(Role::DimText));
        assert!(lum(Role::DimText) > lum(Role::SoftText));
        assert!(lum(Role::SoftText) > lum(Role::Text));
    }

    #[test]
    fn faint_text_on_the_top_bar_meets_the_floor_on_ristretto_and_gruvbox() {
        for (name, want) in [("ristretto", 0x7f7576), ("gruvbox", 0x7c7363)] {
            let theme = on(name);
            assert_eq!(theme.get(Role::FaintText), hex(want), "{name}");
            let faint = colour(&theme, Role::FaintText, &answers(name));
            let bar = colour(&theme, Role::TopBarBackground, &answers(name));
            assert!(contrast(faint, bar) >= FLOOR, "{name}");
        }
    }

    #[test]
    fn the_waiting_dot_on_the_selected_row_meets_the_floor_on_nord() {
        let colors = answers("nord");
        let theme = on("nord");
        assert_eq!(theme.get(Role::WaitingDot), hex(0xcd848b));
        let dot = colour(&theme, Role::WaitingDot, &colors);
        assert!(contrast(dot, colour(&theme, Role::Fill, &colors)) >= FLOOR);
        assert!(passes_hue(Guard::Red, dot));
    }

    #[test]
    fn a_green_role_moved_to_the_floor_is_still_green_on_rose_pine() {
        let colors = answers("rose-pine");
        let theme = on("rose-pine");
        assert_eq!(theme.get(Role::PrOpen), hex(0x6f976b));
        assert_eq!(theme.get(Role::PillOk), hex(0x6f976b));
        assert_eq!(theme.get(Role::StayAwakeDotOn), hex(0x658b62));
        for role in [Role::PrOpen, Role::PillOk, Role::StayAwakeDotOn] {
            let c = colour(&theme, role, &colors);
            assert!(passes_hue(Guard::Green, c), "{}", role.name());
            assert!(lowest(&theme, role, &colors) >= FLOOR, "{}", role.name());
        }
    }

    #[test]
    fn the_terminal_theme_on_white_gives_red_roles_a_red_that_meets_the_floor() {
        let colors = answers("white");
        let theme = on("white");
        assert_eq!(theme.get(Role::PrClosed), hex(0xcb748c));
        assert_eq!(theme.get(Role::PillError), hex(0xcb748c));
        assert_eq!(theme.get(Role::Question), hex(0xcb748c));
        assert_eq!(theme.get(Role::ConfigError), hex(0xbd6c83));
        assert_eq!(theme.get(Role::WaitingDot), hex(0xb06479));
        for role in Role::ALL.iter().filter(|r| r.guard() == Guard::Red) {
            let c = colour(&theme, *role, &colors);
            assert!(passes_hue(Guard::Red, c), "{}", role.name());
            assert!(lowest(&theme, *role, &colors) >= FLOOR, "{}", role.name());
        }
    }

    #[test]
    fn a_grey_red_slot_takes_the_domux_red_on_vantablack() {
        let colors = answers("vantablack");
        assert!(!passes_hue(Guard::Red, colors.palette[1].unwrap()));
        let theme = on("vantablack");
        assert_eq!(theme.get(Role::WaitingDot), hex(0xf38ba8));
        for role in Role::ALL.iter().filter(|r| r.guard() == Guard::Red) {
            assert_eq!(theme.get(*role), hex(0xf38ba8), "{}", role.name());
        }
    }

    #[test]
    fn a_red_slot_that_turns_grey_when_moved_takes_the_next_layers_red() {
        let mut colors = TerminalColors {
            fg: Some(rgb(0xd0d0d0)),
            bg: Some(rgb(0x1a1a1a)),
            palette: [None; 16],
        };
        colors.palette[1] = Some(rgb(0x4a3030));
        assert!(passes_hue(Guard::Red, rgb(0x4a3030)));
        let (theme, trace) = Theme::paint_traced(terminal(), &colors);
        let reds: Vec<Role> = Role::ALL
            .iter()
            .copied()
            .filter(|r| r.guard() == Guard::Red)
            .collect();
        for role in &reds {
            assert_eq!(theme.get(*role), hex(0xf38ba8), "{}", role.name());
        }
        assert_eq!(trace.hue_again, reds);
    }

    #[test]
    fn a_palette_slot_equal_to_the_background_moves_toward_white_until_readable() {
        let chain = chain_of("extends = \"terminal\"\n[roles]\naccent = \"palette 0\"\n");
        let colors = answers("ristretto");
        let (theme, trace) = Theme::paint_traced(&chain, &colors);
        assert_eq!(theme.get(Role::Accent), hex(0x7e7a7a));
        assert_eq!(trace.steps[Role::Accent as usize], 7);
        assert!(lowest(&theme, Role::Accent, &colors) >= FLOOR);
    }

    #[test]
    fn a_light_terminal_with_no_palette_answer_gets_domux_colours_moved_toward_black() {
        let mut colors = answers("catppuccin-latte");
        colors.palette = [None; 16];
        let theme = Theme::paint(terminal(), &colors);
        assert_eq!(theme.get(Role::Accent), hex(0x9378b2));
        assert_eq!(theme.get(Role::HintKey), hex(0x6382b5));
    }

    #[test]
    fn a_hex_value_a_theme_file_wrote_is_never_moved() {
        let chain = chain_of("extends = \"terminal\"\n[roles]\nhint_key = \"#89b4fa\"\n");
        let colors = answers("catppuccin-latte");
        let theme = Theme::paint(&chain, &colors);
        assert_eq!(theme.get(Role::HintKey), hex(0x89b4fa));
        assert!(lowest(&theme, Role::HintKey, &colors) < FLOOR);
    }

    /// The roles the floor holds: the text tiers, the colours, the kinds, the band ends and the
    /// lines.
    fn floored() -> impl Iterator<Item = Role> {
        Role::ALL.iter().copied().filter(|r| {
            matches!(
                r.guard(),
                Guard::TextTier
                    | Guard::Colour
                    | Guard::Red
                    | Guard::Green
                    | Guard::Kind
                    | Guard::Line
            )
        })
    }

    /// Checks that no guarded role reads worse on its grounds than it did before it moved, and
    /// returns the painted theme. A role that took the next layer's value is left out: it did
    /// not move from the value it had.
    fn no_move_lowers_contrast(chain: &Chain, colors: &TerminalColors) -> Theme {
        let (theme, trace) = Theme::paint_traced(chain, colors);
        let unmoved = Theme::paint_unguarded(chain, colors);
        for role in floored().filter(|r| !trace.hue_again.contains(r)) {
            let before = theme.with(role, unmoved.get(role));
            assert!(
                lowest(&theme, role, colors) >= lowest(&before, role, colors),
                "{} moved {} steps from {:.3} to {:.3} on its grounds",
                role.name(),
                trace.steps[role as usize],
                lowest(&before, role, colors),
                lowest(&theme, role, colors),
            );
        }
        theme
    }

    /// The blended tiers, which move together.
    const BLENDED_TIERS: [Role; 4] = [
        Role::SoftText,
        Role::DimText,
        Role::FaintText,
        Role::RecapSeen,
    ];

    #[test]
    fn a_terminal_colour_on_a_hex_ground_is_held_to_the_floor() {
        let chain = chain_of("extends = \"terminal\"\n[roles]\noverlay_background = \"#1e1e2e\"\n");
        let colors = answers("catppuccin-latte");
        let (theme, trace) = Theme::paint_traced(&chain, &colors);
        no_move_lowers_contrast(&chain, &colors);
        assert_eq!(theme.get(Role::OverlayBackground), hex(0x1e1e2e));
        assert_eq!(theme.get(Role::Text), hex(0x6a6c82));
        assert!(lowest(&theme, Role::Text, &colors) >= FLOOR);
        // The blended tiers sit on the dark overlay and on the light top bar, sidebar and tab
        // row. No move meets the floor on both, and every move lowers soft text, so they stay
        // where they are rather than going to white, where they would vanish on the light grounds.
        let unmoved = Theme::paint_unguarded(&chain, &colors);
        for role in BLENDED_TIERS {
            assert_eq!(theme.get(role), unmoved.get(role), "{}", role.name());
            assert_eq!(trace.steps[role as usize], 0, "{}", role.name());
        }
        assert_eq!(theme.get(Role::SoftText), hex(0x707388));
        assert!(lowest(&theme, Role::SoftText, &colors) >= FLOOR);
        // The kinds straddle the same grounds and stay too.
        assert_eq!(theme.get(Role::Codex), hex(0x89b4fa));
    }

    #[test]
    fn a_role_on_a_dark_hex_sidebar_and_a_light_terminal_moves_only_where_it_reads_better() {
        let chain = chain_of("extends = \"terminal\"\n[roles]\nsidebar_background = \"#1e1e2e\"\n");
        let colors = answers("catppuccin-latte");
        let (theme, trace) = Theme::paint_traced(&chain, &colors);
        no_move_lowers_contrast(&chain, &colors);
        // Toward black no step reads on the dark sidebar, so text takes the least step toward
        // white that meets the floor on every ground.
        assert_eq!(theme.get(Role::Text), hex(0x6a6c82));
        assert_eq!(trace.steps[Role::Text as usize], 3);
        assert!(lowest(&theme, Role::Text, &colors) >= FLOOR);
        let unmoved = Theme::paint_unguarded(&chain, &colors);
        for role in BLENDED_TIERS {
            assert_eq!(theme.get(role), unmoved.get(role), "{}", role.name());
        }
        // A kind can meet the floor on both: it moves toward black, as it does on a plain Latte.
        assert_eq!(theme.get(Role::Codex), hex(0x6b8cc2));
        assert!(lowest(&theme, Role::Codex, &colors) >= FLOOR);
        for role in floored() {
            assert_ne!(theme.get(role), hex(0x000000), "{}", role.name());
            assert_ne!(theme.get(role), hex(0xffffff), "{}", role.name());
        }
    }

    #[test]
    fn a_move_that_can_meet_no_floor_takes_the_step_that_reads_best_on_the_worst_ground() {
        let dark = rgb(0x1e1e2e);
        let light = rgb(0xeff1f5);
        let grey = rgb(0x808080);
        // A mid grey on black and white grounds: every step either way lowers one side, so
        // nothing moves.
        assert_eq!(
            best_move(&[(grey, vec![dark, light])], &[BLACK, WHITE], FLOOR),
            (BLACK, 0)
        );
        // Toward the first end first.
        assert_eq!(
            best_move(&[(rgb(0x404040), vec![light])], &[BLACK, WHITE], FLOOR),
            (BLACK, 0)
        );
        let (to, k) = best_move(&[(rgb(0xa0a0a0), vec![light])], &[BLACK, WHITE], FLOOR);
        assert_eq!(to, BLACK);
        assert!(k > 0);
        // When the first end cannot meet the floor and the second can, the least step toward
        // the second.
        let (to, k) = best_move(&[(rgb(0x606060), vec![dark])], &[BLACK, WHITE], FLOOR);
        assert_eq!(to, WHITE);
        assert!(contrast(mix(rgb(0x606060), WHITE, k, STEPS), dark) >= FLOOR);
        assert!(contrast(mix(rgb(0x606060), WHITE, k - 1, STEPS), dark) < FLOOR);
    }

    #[test]
    fn no_move_lowers_a_roles_lowest_contrast_on_any_omarchy_theme() {
        for omarchy in &OMARCHY {
            no_move_lowers_contrast(terminal(), &omarchy.colors());
        }
    }

    #[test]
    fn a_default_ground_never_turns_the_guards_on() {
        let colors = answers("catppuccin-latte");
        let domux = builtin::chain("domux").expect("built in");
        assert_eq!(&Theme::paint(domux, &colors), Theme::domux());

        // text is under the floor on the default sidebar, and still does not move.
        let chain = chain_of("[roles]\nsidebar_background = \"default\"\nfill = \"#ffffff\"\n");
        let theme = Theme::paint(&chain, &colors);
        assert_eq!(theme, Theme::domux().with(Role::Fill, hex(0xffffff)));
        assert!(lowest(&theme, Role::Text, &colors) < FLOOR);
    }

    /// The worked example of the design and of docs/themes.md, byte for byte.
    const RISTRETTO_HEX: &str = r##"# The chrome of Omarchy's Ristretto theme, written out in hex. The kind colours and the band
# are not set, so they keep the domux values.
extends = "domux"

[roles]
# grounds
overlay_background = "#2c2525"
top_bar_background = "#231e1e"
toast_background = "#231e1e"
fill = "#413939"
# On a terminal that is not Ristretto's brown, paint these too:
# sidebar_background = "#2c2525"
# tab_row_background = "#2c2525"

# lines
rule = "#413939"
separator = "#554d4d"
border = "#6a6162"

# text
text = "#e6d9db"
soft_text = "#bdb1b3"
dim_text = "#93898a"
faint_text = "#7f7576"
on_accent = "#2c2525"
on_pill = "#2c2525"

# colours
accent = "#f38d70"
hint_key = "#85dacc"
workspace_name = "#85dacc"
branch = "#a8a9eb"
pr_open = "#adda78"
pr_merged = "#f38d70"
pr_closed = "#fd6883"
pill_ok = "#adda78"
pill_error = "#fd6883"
config_error = "#fd6883"
question = "#fd6883"
waiting_dot = "#fd6883"
stay_awake_dot_on = "#adda78"
stay_awake_dot_off = "#6a6162"
recap = "#e6d9db"
recap_seen = "#bdb1b3"
"##;

    #[test]
    fn the_themes_doc_carries_the_ristretto_hex_example_byte_for_byte() {
        assert!(
            super::super::themes_md::text().contains(&format!("```toml\n{RISTRETTO_HEX}```\n")),
            "docs/themes.md does not carry the Ristretto example byte for byte"
        );
    }

    #[test]
    fn the_ristretto_hex_example_paints_what_terminal_paints_on_ristretto() {
        let chain = chain_of(RISTRETTO_HEX);
        let layer = chain.layers().next().expect("the file's layer");
        assert_eq!(layer.roles.len(), 29);
        let want = on("ristretto");
        let mut all = vec![TerminalColors::default(), answers("catppuccin-latte")];
        all.extend(OMARCHY.iter().map(|t| t.colors()));
        for colors in &all {
            let theme = Theme::paint(&chain, colors);
            for role in layer.roles.keys() {
                assert_eq!(
                    theme.get(*role),
                    want.get(*role),
                    "{} on {colors:?}",
                    role.name()
                );
            }
        }
    }

    #[test]
    fn black_and_white_backgrounds_get_a_top_bar_distinct_from_the_overlay() {
        for (name, want) in [("vantablack", 0x0e0e0e), ("white", 0xf1f1f1)] {
            let colors = answers(name);
            let theme = on(name);
            let over = colour(&theme, Role::OverlayBackground, &colors);
            for role in [Role::TopBarBackground, Role::ToastBackground] {
                assert_eq!(theme.get(role), hex(want), "{name} {}", role.name());
                assert!(contrast(colour(&theme, role, &colors), over) >= GROUND_FLOOR);
            }
        }
    }

    /// The four kinds and the eight band ends, then the four lines.
    const KINDS_AND_LINES: [Role; 16] = [
        Role::Claude,
        Role::Codex,
        Role::Opencode,
        Role::Compacting,
        Role::BandClaudeDim,
        Role::BandClaudeBright,
        Role::BandCodexDim,
        Role::BandCodexBright,
        Role::BandOpencodeDim,
        Role::BandOpencodeBright,
        Role::BandCompactingDim,
        Role::BandCompactingBright,
        Role::Rule,
        Role::Separator,
        Role::Border,
        Role::StayAwakeDotOff,
    ];

    #[test]
    fn the_lines_the_kinds_and_the_band_move_nothing_on_ristretto_or_mocha() {
        let domux = Theme::domux();
        let (mocha, trace) = Theme::paint_traced(terminal(), &answers("catppuccin"));
        for role in KINDS_AND_LINES {
            assert_eq!(mocha.get(role), domux.get(role), "{}", role.name());
            assert_eq!(trace.steps[role as usize], 0, "{}", role.name());
        }
        let (ristretto, trace) = Theme::paint_traced(terminal(), &answers("ristretto"));
        for role in KINDS_AND_LINES {
            assert_eq!(trace.steps[role as usize], 0, "{}", role.name());
        }
        // The kinds and the band keep the domux values; the lines are Ristretto's own blends.
        for role in &KINDS_AND_LINES[..12] {
            assert_eq!(ristretto.get(*role), domux.get(*role), "{}", role.name());
        }
        assert_eq!(ristretto.get(Role::Rule), hex(0x413939));
        assert_eq!(ristretto.get(Role::Separator), hex(0x554d4d));
        assert_eq!(ristretto.get(Role::Border), hex(0x6a6162));
        assert_eq!(ristretto.get(Role::StayAwakeDotOff), hex(0x6a6162));
    }

    #[test]
    fn the_kinds_and_bright_band_ends_on_catppuccin_latte_move_toward_black_to_the_floor() {
        let colors = answers("catppuccin-latte");
        let (theme, trace) = Theme::paint_traced(terminal(), &colors);
        let domux = Theme::domux();
        for (role, want, steps) in [
            (Role::Codex, 0x6b8cc2, 4),
            (Role::Compacting, 0x7e7eb8, 5),
            (Role::Claude, 0xd26d51, 1),
            (Role::BandClaudeBright, 0x9c7b6c, 7),
            (Role::BandClaudeDim, 0xb85e47, 0),
        ] {
            assert_eq!(theme.get(role), hex(want), "{}", role.name());
            assert_eq!(trace.steps[role as usize], steps, "{}", role.name());
            assert!(lowest(&theme, role, &colors) >= FLOOR, "{}", role.name());
        }
        // Toward black: every kind or band end that moved is darker than the domux value.
        for role in &KINDS_AND_LINES[..12] {
            if trace.steps[*role as usize] > 0 {
                let was = colour(domux, *role, &colors);
                assert!(
                    luminance(colour(&theme, *role, &colors)) < luminance(was),
                    "{}",
                    role.name()
                );
            }
        }
        // One step fewer is under the floor, so each moved to the floor and no further.
        let codex = rgb(0x89b4fa);
        let over = colour(&theme, Role::OverlayBackground, &colors);
        assert!(contrast(mix(codex, BLACK, 3, STEPS), over) < FLOOR);
    }

    #[test]
    fn dim_band_ends_move_toward_white_on_nord() {
        let colors = answers("nord");
        let (theme, trace) = Theme::paint_traced(terminal(), &colors);
        assert_eq!(theme.get(Role::BandOpencodeDim), hex(0xaa6f9f));
        assert_eq!(trace.steps[Role::BandOpencodeDim as usize], 2);
        assert_eq!(theme.get(Role::BandCodexDim), hex(0x6d80ad));
        assert_eq!(trace.steps[Role::BandCodexDim as usize], 1);
        for role in [Role::BandOpencodeDim, Role::BandCodexDim] {
            let was = colour(Theme::domux(), role, &colors);
            assert!(luminance(colour(&theme, role, &colors)) > luminance(was));
            assert!(lowest(&theme, role, &colors) >= FLOOR, "{}", role.name());
        }
    }

    #[test]
    fn kinds_and_band_ends_are_not_held_to_the_floor_on_the_selected_row() {
        let colors = answers("catppuccin");
        let theme = on("catppuccin");
        assert_eq!(theme.get(Role::BandOpencodeDim), hex(0x9f5d93));
        let dim = colour(&theme, Role::BandOpencodeDim, &colors);
        let fill = colour(&theme, Role::Fill, &colors);
        assert!(contrast(dim, fill) < FLOOR);
        assert!(lowest(&theme, Role::BandOpencodeDim, &colors) >= FLOOR);
    }

    #[test]
    fn a_kind_colour_a_theme_file_wrote_in_hex_is_never_moved() {
        let chain = chain_of("extends = \"terminal\"\n[roles]\ncodex = \"#89b4fa\"\n");
        let colors = answers("catppuccin-latte");
        let (theme, trace) = Theme::paint_traced(&chain, &colors);
        assert_eq!(theme.get(Role::Codex), hex(0x89b4fa));
        assert_eq!(trace.steps[Role::Codex as usize], 0);
        assert!(lowest(&theme, Role::Codex, &colors) < FLOOR);
        // The kinds the file did not write still move.
        assert_eq!(theme.get(Role::Compacting), hex(0x7e7eb8));
    }

    #[test]
    fn kinds_and_the_band_under_the_domux_theme_never_move_on_a_light_terminal() {
        let colors = answers("catppuccin-latte");
        let domux = builtin::chain("domux").expect("built in");
        let theme = Theme::paint(domux, &colors);
        assert_eq!(theme.get(Role::Codex), hex(0x89b4fa));
        for role in KINDS_AND_LINES {
            assert_eq!(theme.get(role), Theme::domux().get(role), "{}", role.name());
        }

        // A file over domux that reads the answers turns the guards on, and the kinds are still
        // drawn on the domux grounds, so they do not move.
        let chain = chain_of("[roles]\nfill = \"blend 1/9\"\n");
        let (theme, trace) = Theme::paint_traced(&chain, &colors);
        assert_ne!(theme.get(Role::Fill), Theme::domux().get(Role::Fill));
        for role in KINDS_AND_LINES {
            assert_eq!(theme.get(role), Theme::domux().get(role), "{}", role.name());
            assert_eq!(trace.steps[role as usize], 0, "{}", role.name());
        }
    }

    #[test]
    fn the_rule_the_separator_and_the_not_held_dot_meet_their_floors_on_catppuccin_latte() {
        let colors = answers("catppuccin-latte");
        let (theme, trace) = Theme::paint_traced(terminal(), &colors);
        for (role, want, steps) in [
            (Role::Rule, 0xd1d3d8, 1),
            (Role::Separator, 0xc0c2ca, 1),
            (Role::StayAwakeDotOff, 0xa4a6b0, 2),
            (Role::Border, 0xb9bbc6, 0),
        ] {
            assert_eq!(theme.get(role), hex(want), "{}", role.name());
            assert_eq!(trace.steps[role as usize], steps, "{}", role.name());
            let floor = role.floor().expect("a line has a floor");
            assert!(lowest(&theme, role, &colors) >= floor, "{}", role.name());
        }
    }

    #[test]
    fn the_rule_on_the_top_bar_meets_its_floor_on_rose_pine() {
        let colors = answers("rose-pine");
        let (theme, trace) = Theme::paint_traced(terminal(), &colors);
        assert_eq!(theme.get(Role::Rule), hex(0xcec9c7));
        assert_eq!(trace.steps[Role::Rule as usize], 2);
        assert_eq!(theme.get(Role::StayAwakeDotOff), hex(0xa39ea5));
        assert_eq!(trace.steps[Role::StayAwakeDotOff as usize], 3);

        // One step meets the floor on the overlay background but not on the top bar, which is
        // what takes the rule a second step.
        let bg = colors.bg.expect("answered");
        let fg = colors.fg.expect("answered");
        let one = mix(mix(bg, fg, 1, 9), BLACK, 1, STEPS);
        let over = colour(&theme, Role::OverlayBackground, &colors);
        let bar = colour(&theme, Role::TopBarBackground, &colors);
        assert!(contrast(one, over) >= RULE_FLOOR);
        assert!(contrast(one, bar) < RULE_FLOOR);
        let rule = colour(&theme, Role::Rule, &colors);
        assert!(contrast(rule, bar) >= RULE_FLOOR);
    }

    #[test]
    fn the_rule_floor_is_lower_than_the_line_floor() {
        assert_eq!(RULE_FLOOR, 1.25);
        assert_eq!(LINE_FLOOR, 1.4);
        assert_eq!(DOT_OFF_FLOOR, 2.0);
        assert_eq!(Role::Rule.floor(), Some(RULE_FLOOR));
        // The rule on a Mocha terminal is 1.30 on the overlay background: a floor of 1.4 would
        // move it.
        let colors = answers("catppuccin");
        let theme = on("catppuccin");
        let rule = contrast(
            colour(&theme, Role::Rule, &colors),
            colour(&theme, Role::OverlayBackground, &colors),
        );
        assert!((RULE_FLOOR..LINE_FLOOR).contains(&rule), "{rule}");
    }

    #[test]
    fn the_lines_the_kinds_and_the_band_move_the_steps_the_reference_model_names_on_every_omarchy_theme(
    ) {
        use Role::*;
        let brights = |c, x, o, p| {
            vec![
                (BandClaudeBright, c),
                (BandCodexBright, x),
                (BandOpencodeBright, o),
                (BandCompactingBright, p),
            ]
        };
        let dims = vec![
            (BandClaudeDim, 1),
            (BandCodexDim, 1),
            (BandOpencodeDim, 2),
            (BandCompactingDim, 1),
        ];
        let rule = vec![(Rule, 1)];
        let want: [(&str, Vec<(Role, u8)>); 22] = [
            (
                "catppuccin-latte",
                [
                    vec![(Rule, 1), (Separator, 1), (StayAwakeDotOff, 2)],
                    vec![(Claude, 1), (Codex, 4), (Opencode, 1), (Compacting, 5)],
                    brights(7, 7, 6, 7),
                ]
                .concat(),
            ),
            ("catppuccin", vec![]),
            ("ethereal", rule.clone()),
            ("everforest", dims.clone()),
            (
                "flexoki-light",
                [
                    vec![(Rule, 1), (StayAwakeDotOff, 1), (Codex, 4), (Compacting, 4)],
                    brights(6, 6, 5, 6),
                ]
                .concat(),
            ),
            ("gruvbox", vec![]),
            ("hackerman", rule.clone()),
            ("kanagawa", vec![]),
            ("last-horizon", rule.clone()),
            ("lumon", vec![]),
            (
                "lupine",
                [
                    vec![(Rule, 1), (StayAwakeDotOff, 1)],
                    vec![(Codex, 4), (Opencode, 1), (Compacting, 4)],
                    brights(6, 6, 5, 7),
                ]
                .concat(),
            ),
            ("matte-black", rule.clone()),
            ("miasma", vec![]),
            ("nord", dims.clone()),
            ("osaka-jade", vec![]),
            ("retro-82", rule.clone()),
            ("ristretto", vec![]),
            (
                "rose-pine",
                [
                    vec![(Rule, 2), (Separator, 1), (StayAwakeDotOff, 3)],
                    vec![(Claude, 1), (Codex, 4), (Opencode, 1), (Compacting, 4)],
                    brights(6, 7, 6, 7),
                ]
                .concat(),
            ),
            ("solitude", rule.clone()),
            ("tokyo-night", rule.clone()),
            ("vantablack", rule.clone()),
            (
                "white",
                [
                    vec![(Rule, 1), (Codex, 3), (Compacting, 4)],
                    brights(6, 6, 5, 6),
                ]
                .concat(),
            ),
        ];
        for (name, moved) in want {
            let (_, trace) = Theme::paint_traced(terminal(), &answers(name));
            for role in KINDS_AND_LINES {
                let steps = moved
                    .iter()
                    .find(|(r, _)| *r == role)
                    .map_or(0, |(_, k)| *k);
                assert_eq!(trace.steps[role as usize], steps, "{name}: {}", role.name());
            }
        }
    }

    #[test]
    fn every_omarchy_theme_keeps_the_guards_promises() {
        assert_eq!(OMARCHY.len(), 22);
        for omarchy in &OMARCHY {
            let name = omarchy.name;
            let colors = omarchy.colors();
            let (theme, trace) = Theme::paint_traced(terminal(), &colors);
            let c = |role| colour(&theme, role, &colors);

            // The tiers keep their order, toward the far end.
            let over = c(Role::OverlayBackground);
            let dark = luminance(over) < luminance(c(Role::Text));
            let mut tiers: Vec<f64> = [Role::FaintText, Role::DimText, Role::SoftText, Role::Text]
                .map(|r| luminance(c(r)))
                .to_vec();
            if !dark {
                tiers.reverse();
            }
            assert!(tiers.is_sorted(), "{name}: the tiers are out of order");

            for role in Role::ALL {
                match role.guard() {
                    Guard::TextTier | Guard::Colour | Guard::Red | Guard::Green | Guard::Kind => {
                        assert!(
                            lowest(&theme, *role, &colors) >= FLOOR,
                            "{name}: {} is {:.2} on its grounds",
                            role.name(),
                            lowest(&theme, *role, &colors)
                        )
                    }
                    Guard::Line => {
                        let floor = role.floor().expect("a line has a floor");
                        assert!(
                            lowest(&theme, *role, &colors) >= floor,
                            "{name}: {} is {:.4} on its grounds, under {floor}",
                            role.name(),
                            lowest(&theme, *role, &colors)
                        )
                    }
                    Guard::Ground | Guard::Unguarded => {}
                }
                assert!(
                    passes_hue(role.guard(), c(*role)),
                    "{name}: {} is not its colour",
                    role.name()
                );
            }
            // The kinds and the band ends are held on the overlay and the sidebar.
            for role in Role::ALL.iter().filter(|r| r.guard() == Guard::Kind) {
                for ground in [Role::OverlayBackground, Role::SidebarBackground] {
                    assert!(
                        contrast(c(*role), c(ground)) >= FLOOR,
                        "{name}: {} is under the floor on {}",
                        role.name(),
                        ground.name()
                    );
                }
            }
            // The lines keep their order against the overlay background.
            let lines = [Role::Rule, Role::Separator, Role::Border].map(|r| contrast(c(r), over));
            assert!(
                lines[0] < lines[1] && lines[1] < lines[2],
                "{name}: the rule, the separator and the border are out of order: {lines:?}"
            );
            for role in [Role::TopBarBackground, Role::ToastBackground, Role::Fill] {
                assert!(
                    contrast(c(role), over) >= GROUND_FLOOR,
                    "{name}: {} is too close to the overlay background",
                    role.name()
                );
            }
            assert!(
                trace.hue_again.is_empty(),
                "{name}: {:?} needed the second hue pass",
                trace.hue_again
            );
        }
    }
}
