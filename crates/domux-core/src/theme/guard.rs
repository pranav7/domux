//! The readability guards: what keeps a colour nobody chose for this terminal readable on it.
//!
//! They run after a theme is resolved against the terminal's answers, and only when some role's
//! value was read from them. A role moves toward the far end, white on a dark overlay and black on
//! a light one, in steps of 1/18, until it meets its floor on every ground it is drawn on. Moving
//! toward the far end keeps its hue.

use super::color::{contrast, luminance, mix, oklch};
use super::role::{Guard, Role};
use super::value::ColorValue;
use super::Paint;
use domux_term::Rgb;

/// The contrast a text tier or a colour needs on each of its grounds: WCAG 2's minimum for large
/// text and interface marks.
pub const FLOOR: f64 = 3.0;
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
        Guard::Ground | Guard::TextTier | Guard::Colour | Guard::Unguarded => true,
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
        let k = least_steps(&[(from, on)], text, GROUND_FLOOR);
        resolved[role as usize].paint = Paint::Rgb(mix(from, text, k, STEPS));
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
    let k = least_steps(&together, far, FLOOR);
    for (role, (from, _)) in blends.iter().zip(&together) {
        resolved[*role as usize].paint = Paint::Rgb(mix(*from, far, k, STEPS));
        trace.steps[*role as usize] = k;
    }
    for role in tiers {
        if blends.contains(&role) || !movable(resolved, role) {
            continue;
        }
        let from = colour(resolved, role);
        let k = least_steps(&[(from, on(resolved, role))], far, FLOOR);
        resolved[role as usize].paint = Paint::Rgb(mix(from, far, k, STEPS));
        trace.steps[role as usize] = k;
    }

    // 3. Each colour moves on its own. A red or green palette slot that leaves its hue on the
    //    way takes the next layer's value, which is guarded in turn.
    for role in Role::ALL.iter().copied() {
        let guard = role.guard();
        if !matches!(guard, Guard::Colour | Guard::Red | Guard::Green) {
            continue;
        }
        while movable(resolved, role) {
            let from = colour(resolved, role);
            let k = least_steps(&[(from, on(resolved, role))], far, FLOOR);
            let moved = mix(from, far, k, STEPS);
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

/// The least k from 0 to `STEPS` such that every colour, moved k steps toward `to`, meets `floor`
/// against every one of its grounds. `STEPS` when none does.
fn least_steps(colours: &[(Rgb, Vec<Rgb>)], to: Rgb, floor: f64) -> u8 {
    (0..=STEPS)
        .find(|k| {
            colours.iter().all(|(from, on)| {
                let moved = mix(*from, to, *k, STEPS);
                on.iter().all(|ground| contrast(moved, *ground) >= floor)
            })
        })
        .unwrap_or(STEPS)
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

    #[test]
    fn a_terminal_colour_on_a_hex_ground_is_held_to_the_floor() {
        let chain = chain_of("extends = \"terminal\"\n[roles]\noverlay_background = \"#1e1e2e\"\n");
        let theme = Theme::paint(&chain, &answers("catppuccin-latte"));
        assert_eq!(theme.get(Role::OverlayBackground), hex(0x1e1e2e));
        assert_eq!(theme.get(Role::Text), hex(0x6a6c82));
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
                    Guard::TextTier | Guard::Colour | Guard::Red | Guard::Green => assert!(
                        lowest(&theme, *role, &colors) >= FLOOR,
                        "{name}: {} is {:.2} on its grounds",
                        role.name(),
                        lowest(&theme, *role, &colors)
                    ),
                    Guard::Ground | Guard::Unguarded => {}
                }
                assert!(
                    passes_hue(role.guard(), c(*role)),
                    "{name}: {} is not its colour",
                    role.name()
                );
            }
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
