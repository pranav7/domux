//! Every role is drawn where the role table says (design section 7.2, Appendix A).
//!
//! The server draws every client in a probe theme whose 43 roles each have a hex no other role
//! has, so the colour a cell carries names the role it was drawn from. Two roles that share a
//! value under `domux` (`codex` and `hint_key`, `accent` and `pr_merged`, `recap_seen` and
//! `soft_text`) are told apart here and nowhere else.
//!
//! The screens are the surfaces the colour survey lists, reached the way a reader reaches
//! them: keys and API calls against one running server. Each has a table, one row per call
//! site the screen shows: a place, a layer, and the role whose probe value must be there. A
//! failure names the role whose value it found, so `fill` where `rule` belongs says so.

use domux_core::config::{Config, StayAwakeMode};
use domux_core::facts::{Fact, FactKey, FactState, FACT_BRANCH, FACT_PR};
use domux_core::ids::PaneId;
use domux_core::model::agent::AgentKind;
use domux_core::theme::{Paint, Role, Theme};
use domux_server::facts::{FactProvider, FactTarget, ProviderScope};
use domux_server::render::theme::color;
use domux_server::testing::{Harness, HarnessOptions};
use domux_term::Rgb;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);
const COLS: u16 = 140;
const ROWS: u16 = 50;

/// The four band pairs, in the order `band_pair` numbers them.
const BANDS: [(Role, Role); 4] = [
    (Role::BandClaudeDim, Role::BandClaudeBright),
    (Role::BandCodexDim, Role::BandCodexBright),
    (Role::BandOpencodeDim, Role::BandOpencodeBright),
    (Role::BandCompactingDim, Role::BandCompactingBright),
];

/// The band pair `role` belongs to, or `None` for a role that is not a band end.
fn band_pair(role: Role) -> Option<usize> {
    BANDS.iter().position(|(d, b)| *d == role || *b == role)
}

/// A value no other role has. Every role but the band ends sits at green 0x5a and blue 0xa5
/// with its own red. A band end sits at its pair's red, with green and blue both 0x14 at the
/// dim end and 0xe6 at the bright end, so every colour a band draws between its two ends
/// carries its pair's red and cannot be read as any other pair or any other role.
fn probe_rgb(role: Role) -> Rgb {
    match band_pair(role) {
        Some(pair) => {
            let r = 0x0a + 0x3c * pair as u8;
            let level = if BANDS[pair].0 == role { 0x14 } else { 0xe6 };
            Rgb {
                r,
                g: level,
                b: level,
            }
        }
        None => Rgb {
            r: 0x08 + 5 * role as u8,
            g: 0x5a,
            b: 0xa5,
        },
    }
}

fn probe_theme() -> Theme {
    Role::ALL
        .iter()
        .fold(Theme::domux().clone(), |theme, role| {
            theme.with(*role, Paint::Rgb(probe_rgb(*role)))
        })
}

/// What a colour on the screen was drawn from: a role's name, the band pair it sits between,
/// the terminal's default, or the colour itself when it is none of those.
fn name_of(theme: &Theme, found: Color) -> String {
    if found == Color::Reset {
        return "the terminal's default".into();
    }
    if let Some(role) = Role::ALL.iter().find(|r| color(theme, **r) == found) {
        return role.name().to_string();
    }
    if let Some(pair) = band_on(found) {
        return format!(
            "a colour between {} and {}",
            BANDS[pair].0.name(),
            BANDS[pair].1.name()
        );
    }
    format!("{found:?}, which is no role")
}

/// The band pair whose two ends `found` sits between, if any.
fn band_on(found: Color) -> Option<usize> {
    let Color::Rgb(r, g, b) = found else {
        return None;
    };
    (0..BANDS.len()).find(|pair| {
        let dim = probe_rgb(BANDS[*pair].0);
        let bright = probe_rgb(BANDS[*pair].1);
        r == dim.r && g == b && (dim.g..=bright.g).contains(&g)
    })
}

/// Where a row of the table looks: the `nth` place (from zero) `text` is drawn, reading rows
/// top to bottom and only rows that also hold `row` when there is one, then `skip` cells
/// further right. Or one cell by column and row, when `text` is empty.
#[derive(Debug, Clone, Copy)]
struct At {
    row: Option<&'static str>,
    text: &'static str,
    nth: usize,
    skip: u16,
    cell: (u16, u16),
}

const fn text(text: &'static str) -> At {
    At {
        row: None,
        text,
        nth: 0,
        skip: 0,
        cell: (0, 0),
    }
}

const fn cell(x: u16, y: u16) -> At {
    At {
        row: None,
        text: "",
        nth: 0,
        skip: 0,
        cell: (x, y),
    }
}

impl At {
    const fn in_row(self, row: &'static str) -> At {
        At {
            row: Some(row),
            ..self
        }
    }

    const fn skip(self, skip: u16) -> At {
        At { skip, ..self }
    }

    const fn nth(self, nth: usize) -> At {
        At { nth, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Fg,
    Bg,
    /// The foreground of every cell from the place up to and including the next `…`, which is
    /// a working word under its band: each must sit between the role's pair of band ends, and
    /// the least lit of them must be nearer the dim end, so a band drawn backwards fails too.
    Band,
}

struct Probe {
    place: &'static str,
    at: At,
    layer: Layer,
    role: Role,
}

const fn fg(place: &'static str, at: At, role: Role) -> Probe {
    Probe {
        place,
        at,
        layer: Layer::Fg,
        role,
    }
}

const fn bg(place: &'static str, at: At, role: Role) -> Probe {
    Probe {
        place,
        at,
        layer: Layer::Bg,
        role,
    }
}

const fn band(place: &'static str, at: At, role: Role) -> Probe {
    Probe {
        place,
        at,
        layer: Layer::Band,
        role,
    }
}

/// Each screen row as its cells' text and the column each cell starts in. A wide glyph's
/// spacer draws nothing and is left out, so a column is the grid's own.
fn cells(buf: &Buffer) -> Vec<Vec<(String, u16)>> {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .filter_map(|x| {
                    let s = buf[(x, y)].symbol();
                    (!s.is_empty()).then(|| (s.to_string(), x))
                })
                .collect()
        })
        .collect()
}

/// Every place `text` starts on row `y`, as columns.
fn starts_in(row: &[(String, u16)], text: &str) -> Vec<u16> {
    let want: Vec<String> = text.chars().map(|c| c.to_string()).collect();
    if want.is_empty() || row.len() < want.len() {
        return Vec::new();
    }
    (0..=row.len() - want.len())
        .filter(|i| (0..want.len()).all(|k| row[i + k].0 == want[k]))
        .map(|i| row[i].1)
        .collect()
}

fn locate(buf: &Buffer, at: At) -> Option<(u16, u16)> {
    if at.text.is_empty() {
        return Some(at.cell);
    }
    let rows = cells(buf);
    let (x, y) = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| at.row.is_none_or(|needle| !starts_in(r, needle).is_empty()))
        .flat_map(|(y, r)| {
            starts_in(r, at.text)
                .into_iter()
                .map(move |x| (x, y as u16))
        })
        .nth(at.nth)?;
    Some((x + at.skip, y))
}

/// The failures on this screen, one sentence each.
fn check(theme: &Theme, screen: &str, buf: &Buffer, table: &[Probe]) -> Vec<String> {
    let mut out = Vec::new();
    for probe in table {
        let Some((x, y)) = locate(buf, probe.at) else {
            out.push(format!(
                "{screen}: {}: {:?} is not on the screen",
                probe.place, probe.at
            ));
            continue;
        };
        let want = color(theme, probe.role);
        match probe.layer {
            Layer::Fg | Layer::Bg => {
                let cell = &buf[(x, y)];
                let found = if probe.layer == Layer::Fg {
                    cell.fg
                } else {
                    cell.bg
                };
                if found != want {
                    out.push(format!(
                        "{screen}: {} at c{x} r{y} ({:?}) is {} where the table says {}",
                        probe.place,
                        probe.layer,
                        name_of(theme, found),
                        probe.role.name()
                    ));
                }
            }
            Layer::Band => {
                let pair = band_pair(probe.role).expect("a band row names a band end");
                let dim = probe_rgb(BANDS[pair].0);
                let bright = probe_rgb(BANDS[pair].1);
                let mut least = u8::MAX;
                let mut drawn = 0;
                for cx in x..buf.area.width {
                    let cell = &buf[(cx, y)];
                    if cell.symbol().is_empty() {
                        continue;
                    }
                    drawn += 1;
                    match (band_on(cell.fg), cell.fg) {
                        (Some(p), Color::Rgb(_, g, _)) if p == pair => least = least.min(g),
                        _ => {
                            out.push(format!(
                                "{screen}: {} at c{cx} r{y} is {} where the table says a colour between {} and {}",
                                probe.place,
                                name_of(theme, cell.fg),
                                BANDS[pair].0.name(),
                                BANDS[pair].1.name()
                            ));
                            break;
                        }
                    }
                    if cell.symbol() == "…" {
                        break;
                    }
                }
                // `render::shimmer` lights no character less than 0.35 of the way to the
                // bright end, and a word is long enough that some character is always that
                // far from the band. So drawn the right way round, the least lit character
                // sits between 0.3 of the way and half way. Drawn backwards it sits either
                // under the band, nearer the dim end, or past half way when the band is off the
                // word, which is why `screen` looks at more than one frame.
                let span = f64::from(bright.g - dim.g);
                let lowest = f64::from(least.saturating_sub(dim.g)) / span;
                if drawn < 3 || !(0.3..0.5).contains(&lowest) {
                    out.push(format!(
                        "{screen}: {} is lit {lowest:.2} of the way from {} to {} at its least, over {drawn} cells, so it does not run from the one to the other",
                        probe.place,
                        BANDS[pair].0.name(),
                        BANDS[pair].1.name()
                    ));
                }
            }
        }
    }
    out
}

/// Draws the frame after the server has had time to send it, waits for `ready`, and checks
/// `table` against it. The frame is printed with a failure, so the place can be read.
async fn screen(
    h: &mut Harness,
    theme: &Theme,
    name: &str,
    ready: impl Fn(&str) -> bool,
    table: &[Probe],
    failures: &mut Vec<String>,
    covered: &mut BTreeSet<Role>,
) {
    let mut frame = h.wait_for(h.client.clone(), ready, WAIT).await;
    let mut found = check(theme, name, &h.buffer(&h.client), table);
    // A band moves every tick, so a screen with one is read a few ticks apart: where the band
    // sits changes what one frame can say about which way it runs.
    if table.iter().any(|p| p.layer == Layer::Band) {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            frame = h.frame(h.client.clone()).await;
            for failure in check(theme, name, &h.buffer(&h.client), table) {
                if !found.contains(&failure) {
                    found.push(failure);
                }
            }
        }
    }
    if !found.is_empty() {
        // The rows, without the style dump: every cell is named in the sentences above.
        let rows: Vec<&str> = frame.lines().filter(|l| l.starts_with('|')).collect();
        failures.push(format!("{}\n{}", found.join("\n"), rows.join("\n")));
    }
    for probe in table {
        covered.insert(probe.role);
        if probe.layer == Layer::Band {
            let pair = band_pair(probe.role).unwrap();
            covered.insert(BANDS[pair].0);
            covered.insert(BANDS[pair].1);
        }
    }
}

/// A pull request per slot, by the slot's number: open, merged, closed and draft, and none on
/// the fifth, which keeps it untouched. The title rides in `url`, as the real provider's does.
struct ProbePrs;

impl FactProvider for ProbePrs {
    fn name(&self) -> &str {
        FACT_PR
    }
    fn interval(&self) -> Duration {
        Duration::from_millis(50)
    }
    fn scope(&self) -> ProviderScope {
        ProviderScope::Workspace
    }
    fn fetch(&self, target: &FactTarget) -> Result<Option<Fact>, String> {
        let (number, state) = match slot(target) {
            Some(1) => (11, FactState::Open),
            Some(2) => (12, FactState::Merged),
            Some(3) => (13, FactState::Closed),
            Some(4) => (14, FactState::Draft),
            _ => return Ok(None),
        };
        let mut fact = Fact::new(
            format!("PR#{number}"),
            Some(state),
            target.now.to_rfc3339(),
            Duration::from_secs(600),
        );
        fact.url = Some(format!("Probe title {number}"));
        Ok(Some(fact))
    }
}

/// A branch per slot that line 2 draws, and the fifth slot on its own handle, which with no
/// pull request is what makes a slot untouched.
struct ProbeBranches;

impl FactProvider for ProbeBranches {
    fn name(&self) -> &str {
        FACT_BRANCH
    }
    fn interval(&self) -> Duration {
        Duration::from_millis(50)
    }
    fn scope(&self) -> ProviderScope {
        ProviderScope::Workspace
    }
    fn fetch(&self, target: &FactTarget) -> Result<Option<Fact>, String> {
        match slot(target) {
            Some(n) => Ok(Some(Fact::new(
                match n {
                    5 => "workspace-5".to_string(),
                    _ => format!("feat/probe-{n}"),
                },
                None,
                target.now.to_rfc3339(),
                Duration::from_secs(600),
            ))),
            None => Ok(None),
        }
    }
}

fn slot(target: &FactTarget) -> Option<u32> {
    target
        .path
        .file_name()?
        .to_str()?
        .strip_prefix("workspace-")?
        .parse()
        .ok()
}

/// A new tab in the client's workspace, and the pane it opened with.
async fn new_pane(h: &mut Harness) -> PaneId {
    h.api("tab.create", json!({})).await.expect("tab.create");
    h.frame(h.client.clone()).await;
    h.focused_pane(h.client.clone())
}

/// A transcript that names the session, and holds a recap when there is one, for the tick to
/// read.
fn transcript(dir: &std::path::Path, name: &str, recap: Option<&str>) -> String {
    let path = dir.join(format!("{name}.jsonl"));
    let mut lines = format!(
        "{{\"type\":\"custom-title\",\"customTitle\":\"{name}\",\"sessionId\":\"{name}\"}}\n"
    );
    if let Some(recap) = recap {
        lines.push_str(&format!(
            "{{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"{recap}\"}}\n"
        ));
    }
    std::fs::write(&path, lines).unwrap();
    path.to_str().unwrap().to_string()
}

async fn claude(h: &mut Harness, pane: &PaneId, session: &str, event: &str, path: Option<&str>) {
    let mut payload = json!({"hook_event_name": event, "session_id": session});
    if let Some(path) = path {
        payload["transcript_path"] = json!(path);
    }
    h.report(pane.clone(), AgentKind::Claude, &payload.to_string())
        .await;
}

#[tokio::test]
async fn every_role_is_drawn_where_the_role_table_says() {
    use Role::*;
    let theme = probe_theme();
    let values: std::collections::HashSet<Rgb> = Role::ALL.iter().map(|r| probe_rgb(*r)).collect();
    assert_eq!(
        values.len(),
        Role::ALL.len(),
        "every probe value is its own"
    );

    let mut config = Config::default();
    config.stay_awake.mode = StayAwakeMode::Full;
    config
        .keys
        .bindings
        .insert("D".into(), "workspace.delete workspace-1".into());
    config
        .keys
        .bindings
        .insert("C".into(), "workspace.clear workspace-2".into());
    let mut h = Harness::start_with(HarnessOptions {
        theme: Some(theme.clone()),
        platform: Some("macos"),
        providers: vec![Arc::new(ProbePrs), Arc::new(ProbeBranches)],
        ..HarnessOptions::new(config, COLS, ROWS)
    })
    .await;
    h.runner.on_path("caffeinate");
    // Full mode's lid needs sudo, and a sudo that says no is what puts a second line in the
    // toast.
    h.runner
        .fail_starting_with("sudo", &["-n"], "sudo needs a password");
    let client = h.client.clone();
    let mut failures = Vec::new();
    let mut covered = BTreeSet::new();

    // ---- The top bar, and the tab row on it.
    h.api("tab.create", json!({})).await.unwrap();
    h.api("tab.select", json!({"tab": "1"})).await.unwrap();
    screen(
        &mut h,
        &theme,
        "the top bar",
        |f| f.contains(" 2 "),
        &[
            bg("the top bar's row", cell(COLS / 2, 0), TopBarBackground),
            fg("the location label", text("proj"), Text),
            fg(
                "the current tab while keys go to a pane",
                text(" 1 ").in_row("proj"),
                OnAccent,
            ),
            bg("the accent fill", text(" 1 ").in_row("proj"), Accent),
            fg("another tab", text("2").in_row("proj"), DimText),
            fg("the rule between tabs", text("│").in_row("proj"), Rule),
            fg("the new tab mark", text("+").in_row("proj"), Border),
            fg("the clock", text("14:32"), SoftText),
            fg("the stay awake dot while off", text("●"), StayAwakeDotOff),
            fg("the focused region's border", cell(0, 1), Accent),
            fg("the focused region's title", text("sh").in_row("┌"), Accent),
        ],
        &mut failures,
        &mut covered,
    )
    .await;

    h.key(client.clone(), "C-s").await;
    screen(
        &mut h,
        &theme,
        "the chord in the top bar",
        |f| f.contains("C-s  ? keys"),
        &[
            fg("the chord's leader", text("C-s  ?"), HintKey),
            fg("the chord's word", text("C-s  ?").skip(7), FaintText),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "[").await;
    screen(
        &mut h,
        &theme,
        "copy mode",
        |f| f.contains("v select · ⏎ copy · esc leave"),
        &[
            fg("a copy mode key", text("v select"), HintKey),
            fg("a copy mode word", text("v select").skip(2), FaintText),
            fg(
                "the copy mode separator",
                text("v select").skip(9),
                Separator,
            ),
            fg(
                "the focused region's flag",
                text("copy").in_row("┌ sh"),
                Accent,
            ),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("v select"), WAIT)
        .await;

    h.api("tab.rename", json!({})).await.unwrap();
    screen(
        &mut h,
        &theme,
        "the tab name prompt",
        |f| f.contains("⏎ save · esc cancel"),
        &[
            fg("the prompt's text", text("Name tab 1"), OnAccent),
            bg("the prompt's fill", text("Name tab 1"), Accent),
            fg("a prompt key in the top bar", text("⏎ save"), HintKey),
            fg(
                "a prompt word in the top bar",
                text("⏎ save").skip(2),
                FaintText,
            ),
            fg("the top bar's separator", text("⏎ save").skip(7), Separator),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("⏎ save"), WAIT)
        .await;

    h.api("stay_awake.enable", json!({})).await.unwrap();
    screen(
        &mut h,
        &theme,
        "stay awake and its toast",
        |f| f.contains("Stay awake turned on"),
        &[
            fg("the stay awake dot while on", text("●"), StayAwakeDotOn),
            bg(
                "the toast's ground",
                text("Stay awake turned on"),
                ToastBackground,
            ),
            fg("the toast's first line", text("Stay awake turned on"), Text),
            fg(
                "a later line of the toast",
                text("the machine is held"),
                SoftText,
            ),
            fg("the toast's border", text("│ Stay awake turned on"), Border),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.advance(Duration::from_secs(10));
    h.wait_for(
        client.clone(),
        |f| !f.contains("Stay awake turned on"),
        WAIT,
    )
    .await;

    // ---- A git project with five slots, and an agent in every state in `proj`'s `main`.
    let root = h.git_project("main").await;
    let project = h
        .model()
        .project_at(&root.canonicalize().unwrap())
        .map(|p| p.id.to_string())
        .unwrap();
    for _ in 0..5 {
        h.api("workspace.create", json!({"project": project}))
            .await
            .expect("workspace.create");
    }
    let dir = tempfile::tempdir().unwrap();
    let working = transcript(dir.path(), "probe-work", None);
    let busy = transcript(dir.path(), "probe-compact", Some("Probe recap busy."));
    let seen = transcript(dir.path(), "probe-idle", Some("Probe recap seen."));

    let pane = new_pane(&mut h).await;
    claude(&mut h, &pane, "c-work", "SessionStart", Some(&working)).await;
    claude(&mut h, &pane, "c-work", "UserPromptSubmit", Some(&working)).await;
    let pane = new_pane(&mut h).await;
    h.report(
        pane.clone(),
        AgentKind::Codex,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"x-work"}"#,
    )
    .await;
    let pane = new_pane(&mut h).await;
    for event in ["session.created", "message.updated"] {
        h.report_raw(
            pane.clone(),
            AgentKind::Opencode,
            &json!({"hook_event_name": event, "session_id": "o-work"}).to_string(),
        )
        .await;
    }
    let pane = new_pane(&mut h).await;
    claude(&mut h, &pane, "c-compact", "SessionStart", Some(&busy)).await;
    claude(&mut h, &pane, "c-compact", "PreCompact", Some(&busy)).await;
    let pane = new_pane(&mut h).await;
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"c-wait","notification_type":"permission_prompt","message":"m"}"#,
    )
    .await;
    let pane = new_pane(&mut h).await;
    claude(&mut h, &pane, "c-idle", "SessionStart", Some(&seen)).await;
    let pane = new_pane(&mut h).await;
    h.set_foreground_for(&pane, Some("claude")).await;
    h.api("tab.select", json!({"tab": "1"})).await.unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let m = h.model();
        let named = m.agents.iter().filter(|a| a.name.is_some()).count();
        let recaps = m.agents.iter().filter(|a| a.recap.is_some()).count();
        let prs = m
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .filter(|w| h.fact(&FactKey::workspace(&w.id, FACT_PR)).is_some())
            .count();
        if m.agents.len() == 7 && named == 3 && recaps == 2 && prs == 4 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the agents and the facts did not all arrive: {prs} pull requests, {:?}",
            m.agents
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // ---- The switcher: a pill first, then every pull request state, every agent state and
    // the fill.
    h.api("switcher.open", json!({})).await.unwrap();
    screen(
        &mut h,
        &theme,
        "the switcher's pill",
        |f| f.contains("Created workspace-5"),
        &[
            fg(
                "an ok pill's text in a footer",
                text("Created workspace-5"),
                OnPill,
            ),
            bg(
                "an ok pill in a footer",
                text("Created workspace-5"),
                PillOk,
            ),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.advance(Duration::from_secs(10));
    screen(
        &mut h,
        &theme,
        "the switcher",
        |f| f.contains("Probe recap seen") && f.contains("PR#14") && f.contains("⏎ open"),
        &[
            bg(
                "the overlay's ground",
                text("AUDREY-APP"),
                OverlayBackground,
            ),
            fg("the focused box's title", text("Navigator"), Accent),
            fg("the focused box's border", text("┌ Navigator"), Accent),
            fg("a project header", text("AUDREY-APP"), DimText),
            fg(
                "the rule under a project header",
                text("AUDREY-APP").skip(11),
                Rule,
            ),
            fg("main unfilled", text("│    main").skip(5), DimText),
            fg(
                "a live workspace's name",
                text("workspace-1"),
                WorkspaceName,
            ),
            fg("an untouched slot", text("◌ workspace-5"), FaintText),
            fg("a branch", text("feat/probe-1"), Branch),
            fg(
                "the separator on line 2",
                text("feat/probe-1").skip(13),
                Separator,
            ),
            fg("an open pull request", text("PR#11"), PrOpen),
            fg("a merged pull request", text("PR#12"), PrMerged),
            fg("a closed pull request", text("PR#13"), PrClosed),
            fg("a draft pull request", text("PR#14"), DimText),
            fg(
                "the separator before a title",
                text("PR#11").skip(6),
                Separator,
            ),
            fg("a pull request title", text("Probe title 11"), DimText),
            bg("the selected row", text("│    main").nth(1).skip(5), Fill),
            bg(
                "the fill across the row",
                text("│    main").nth(1).skip(1),
                Fill,
            ),
            fg(
                "main on the selected row",
                text("│    main").nth(1).skip(5),
                Text,
            ),
            fg("the nest corner", text("└"), FaintText),
            fg(
                "a kind standing in for a name",
                text("claude").in_row("◉"),
                Claude,
            ),
            fg("the waiting dot", text("◉"), WaitingDot),
            fg("a session name", text("probe-work"), Text),
            fg(
                "a claude working glyph",
                text("probe-work").skip(11),
                Claude,
            ),
            band(
                "a claude working word",
                text("probe-work").skip(13),
                BandClaudeDim,
            ),
            fg("a codex label", text("codex"), Codex),
            fg("a codex working glyph", text("codex").skip(6), Codex),
            band("a codex working word", text("codex").skip(8), BandCodexDim),
            fg("an opencode label", text("opencode").in_row("…"), Opencode),
            fg(
                "an opencode working glyph",
                text("opencode").in_row("…").skip(9),
                Opencode,
            ),
            band(
                "an opencode working word",
                text("opencode").in_row("…").skip(11),
                BandOpencodeDim,
            ),
            fg(
                "the compacting glyph",
                text("probe-compact").skip(14),
                Compacting,
            ),
            band(
                "the compacting word",
                text("Compacting…"),
                BandCompactingDim,
            ),
            fg(
                "the kind in the tail",
                text("claude").in_row("probe-compact"),
                Claude,
            ),
            fg(
                "the separator in the tail",
                text("›").in_row("probe-compact"),
                Separator,
            ),
            fg(
                "the tab in the tail",
                text("t_").in_row("probe-compact"),
                DimText,
            ),
            fg(
                "the pane in the tail",
                text("sh").in_row("probe-compact"),
                DimText,
            ),
            fg(
                "an unknown agent's name",
                text("claude").in_row("unknown"),
                FaintText,
            ),
            fg("unknown", text("unknown"), FaintText),
            fg("a recap on a busy row", text("Probe recap busy"), Recap),
            fg("a recap on a seen row", text("Probe recap seen"), RecapSeen),
            fg("the footer's key", text("⏎ open"), HintKey),
            fg("the footer's word", text("⏎ open").skip(2), FaintText),
            fg("the footer's separator", text("⏎ open").skip(7), Separator),
            bg("the footer's ground", text("⏎ open"), OverlayBackground),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "/").await;
    h.type_text(client.clone(), "probe").await;
    screen(
        &mut h,
        &theme,
        "the switcher's filter",
        |f| f.contains("Filter › probe"),
        &[
            fg("the filter's word in a footer", text("Filter ›"), FaintText),
            fg(
                "the filter's text in a footer",
                text("Filter ›").skip(9),
                Text,
            ),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.api("switcher.close", json!({})).await.unwrap();
    h.wait_for(client.clone(), |f| !f.contains("┌ Navigator"), WAIT)
        .await;

    // ---- The agents overlay.
    h.api("agents.open", json!({})).await.unwrap();
    screen(
        &mut h,
        &theme,
        "the agents overlay",
        |f| f.contains("┌ Agents") && f.contains("Probe recap seen"),
        &[
            fg("a project header", text("PROJ"), DimText),
            fg("the rule under it", text("PROJ").skip(5), Rule),
            fg("a kind on line 2", text("claude ·"), Claude),
            fg(
                "the separator on line 2",
                text("claude ·").skip(7),
                Separator,
            ),
            fg(
                "the place on line 2",
                text("claude · main").skip(9),
                DimText,
            ),
            fg("the pane on line 2", text("› sh").nth(0).skip(2), DimText),
            fg("a recap", text("Probe recap busy"), Recap),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.api("agents.close", json!({})).await.unwrap();
    h.wait_for(client.clone(), |f| !f.contains("┌ Agents"), WAIT)
        .await;

    // ---- Keys.
    h.api("help", json!({})).await.unwrap();
    screen(
        &mut h,
        &theme,
        "Keys",
        |f| f.contains("┌ Keys"),
        &[
            fg("the leader", text("leader C-s"), HintKey),
            fg("the legend", text("C Ctrl"), SoftText),
            fg("a header", text("workpanel"), Text),
            fg("a body line", text("switcher.open"), Text),
            fg("the footer", text("esc close"), HintKey),
            bg(
                "the overlay's ground",
                text("leader C-s"),
                OverlayBackground,
            ),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("┌ Keys"), WAIT)
        .await;

    // ---- The name box: its hint row, a typed name, and a refused name.
    let w1 = h
        .model()
        .projects
        .iter()
        .flat_map(|p| p.workspaces.iter())
        .find(|w| w.handle.to_string() == "workspace-1")
        .map(|w| w.id.clone())
        .unwrap();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    h.key(client.clone(), "C-s").await;
    h.key(client.clone(), "N").await;
    screen(
        &mut h,
        &theme,
        "the name box",
        |f| f.contains("Name workspace-1") && f.contains("an empty name clears it"),
        &[
            fg("the name box's key", text("⏎ save"), HintKey),
            fg("the name box's word", text("an empty name"), FaintText),
            bg("the name box's ground", text("⏎ save"), OverlayBackground),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.type_text(client.clone(), "workspace-9").await;
    screen(
        &mut h,
        &theme,
        "the name box's field",
        |f| f.contains("│ workspace-9"),
        &[fg("the typed name", text("│ workspace-9").skip(2), Text)],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Enter").await;
    screen(
        &mut h,
        &theme,
        "the name box's refusal",
        |f| f.contains("is a handle"),
        &[
            fg(
                "a refused pill's text",
                text("workspace-9 is a handle"),
                OnPill,
            ),
            bg("a refused pill", text("workspace-9 is a handle"), PillError),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("┌ Name"), WAIT)
        .await;

    // ---- The three confirmations.
    h.key(client.clone(), "C-s").await;
    h.key(client.clone(), "D").await;
    screen(
        &mut h,
        &theme,
        "the delete question",
        |f| f.contains("Delete workspace-1?"),
        &[
            fg("the question", text("Delete workspace-1?"), Question),
            fg("which workspace", text("worktrees/workspace-1"), DimText),
            fg("what goes", text("Removes"), Text),
            fg("what stays", text("The remote branch"), FaintText),
            fg("the answer key", text("y delete"), HintKey),
            fg("what the answer does", text("y delete").skip(2), Text),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("Delete workspace-1?"), WAIT)
        .await;
    h.key(client.clone(), "C-s").await;
    h.key(client.clone(), "C").await;
    screen(
        &mut h,
        &theme,
        "the clear question",
        |f| f.contains("Clear workspace-2?"),
        &[
            fg("the question", text("Clear workspace-2?"), Question),
            fg("which workspace", text("worktrees/workspace-2"), DimText),
            fg("what goes", text("Throws away"), Text),
            fg(
                "what keeps running",
                text("Nothing in its panes"),
                FaintText,
            ),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("Clear workspace-2?"), WAIT)
        .await;
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(client.clone(), |f| f.contains("┌ Navigator"), WAIT)
        .await;
    h.key(client.clone(), "X").await;
    screen(
        &mut h,
        &theme,
        "the remove question",
        |f| f.contains("Remove audrey-app?"),
        &[
            fg("the question", text("Remove audrey-app?"), Question),
            fg("what goes", text("Removes the record"), Text),
            fg(
                "what stays",
                text("The folder and its worktrees"),
                FaintText,
            ),
            fg("the answer key", text("y remove"), HintKey),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("Remove audrey-app?"), WAIT)
        .await;
    h.api("switcher.close", json!({})).await.unwrap();
    h.wait_for(client.clone(), |f| !f.contains("┌ Navigator"), WAIT)
        .await;

    // ---- The sidebar, the tab row on the panes, the hint row, the filter and a pill.
    h.api("sidebar.show", json!({})).await.unwrap();
    screen(
        &mut h,
        &theme,
        "the sidebar",
        |f| f.contains("┌ Navigator") && f.contains("leader b hide"),
        &[
            bg(
                "the sidebar's border",
                text("─").in_row("┌ Navigator"),
                SidebarBackground,
            ),
            bg(
                "a row in the sidebar",
                text("feat/probe-1"),
                SidebarBackground,
            ),
            bg(
                "a blank cell in the sidebar",
                text("│ AUDREY-APP").skip(1),
                SidebarBackground,
            ),
            bg(
                "the tab row on the panes",
                text("+").in_row("┌ Navigator"),
                TabRowBackground,
            ),
            bg(
                "a blank cell between the tabs and the right end",
                text("+").in_row("┌ Navigator").skip(4),
                TabRowBackground,
            ),
            bg(
                "the tab row's last column",
                cell(COLS - 1, 0),
                TabRowBackground,
            ),
            fg("an unfocused box's border", text("┌ Navigator"), Border),
            fg(
                "an unfocused box's title",
                text("┌ Navigator").skip(2),
                DimText,
            ),
            fg("the nest corner", text("└"), FaintText),
            fg("the hint row's key", text("leader b"), HintKey),
            fg("the hint row's word", text("leader b").skip(9), FaintText),
            fg(
                "the hint row's separator",
                text("leader b hide").skip(14),
                Separator,
            ),
            bg("the hint row", text("leader b"), SidebarBackground),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "C-h").await;
    h.wait_for(client.clone(), |f| f.contains("? more"), WAIT)
        .await;
    h.key(client.clone(), "/").await;
    h.type_text(client.clone(), "probe").await;
    screen(
        &mut h,
        &theme,
        "the sidebar's filter",
        |f| f.contains("Filter › probe"),
        &[
            fg("the filter's word", text("Filter ›"), FaintText),
            fg("the filter's text", text("Filter ›").skip(9), Text),
        ],
        &mut failures,
        &mut covered,
    )
    .await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("Filter ›"), WAIT)
        .await;
    h.key(client.clone(), "n").await;
    h.wait_for(client.clone(), |f| f.contains("┌ Name"), WAIT)
        .await;
    h.type_text(client.clone(), "probe-name").await;
    h.key(client.clone(), "Enter").await;
    screen(
        &mut h,
        &theme,
        "the sidebar's pill",
        |f| f.contains("Named workspace-1 probe-name"),
        &[
            fg("an ok pill's text", text("Named workspace-1"), OnPill),
            bg("an ok pill", text("Named workspace-1"), PillOk),
        ],
        &mut failures,
        &mut covered,
    )
    .await;

    // ---- Last, because a file that does not parse keeps the config but not the keys this
    // test bound: the config error.
    h.api("sidebar.hide", json!({})).await.unwrap();
    std::fs::write(h.config_path(), "[keys\nleader = 1\n").unwrap();
    let _ = h.api("config.reload", json!({})).await;
    screen(
        &mut h,
        &theme,
        "the config error",
        |f| f.contains("domux.toml"),
        &[fg("the config error", text("domux.toml"), ConfigError)],
        &mut failures,
        &mut covered,
    )
    .await;

    let missing: Vec<&str> = Role::ALL
        .iter()
        .filter(|r| !covered.contains(r))
        .map(|r| r.name())
        .collect();
    assert!(missing.is_empty(), "no row checks {missing:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
