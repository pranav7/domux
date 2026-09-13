//! Themes chosen in `domux.toml` and written under `themes/` (design sections 3.4, 3.5 and
//! 7.2): a theme file is drawn from the start and after a reload, a broken one warns, and a
//! reload that breaks it keeps the theme drawn before.
//!
//! Each client is drawn from its own terminal's colours and desktop (sections 5.4 and 7.2): the
//! terminal theme reads what the terminal answered, `auto` reads the desktop, a colours message
//! recolours the chrome, and each client is told whether to follow its terminal's colours.

use domux_core::config::Config;
use domux_core::ids::ClientId;
use domux_core::model::agent::AgentKind;
use domux_core::proto::Capabilities;
use domux_core::theme::{Desktop, TerminalColors};
use domux_server::load_config;
use domux_server::testing::{self, Harness};
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use serde_json::json;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

/// The Ristretto look in hex, word for word from design section 3.3.
const RISTRETTO: &str = r##"# The chrome of Omarchy's Ristretto theme, written out in hex. The kind colours and the band
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

/// A theme file toml cannot read: the table header is never closed.
const BROKEN: &str = "extends = \"domux\"\n[roles\ntext = \"#e6d9db\"\n";

/// The overlay's ground under Ristretto and under domux.
const RISTRETTO_OVERLAY: &str = "bg=#2c2525";
const DOMUX_OVERLAY: &str = "bg=#1e1e2e";

fn name_theme(h: &Harness, name: &str) {
    std::fs::write(h.config_path(), format!("[theme]\nname = \"{name}\"\n")).unwrap();
}

/// Opens the switcher and answers its frame once it is drawn in `ground`.
async fn switcher_in(h: &mut Harness, ground: &str) -> String {
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Navigator") && f.contains(ground),
            WAIT,
        )
        .await;
    h.api("switcher.close", json!({})).await.unwrap();
    h.wait_for(h.client.clone(), |f| !f.contains("┌ Navigator"), WAIT)
        .await;
    f
}

#[tokio::test]
async fn a_theme_file_named_in_the_config_is_drawn_at_start() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.write_theme("ristretto", RISTRETTO);
    name_theme(&h, "ristretto");
    // A new server on the same state directory, so the theme is what it loads at start.
    h.restart().await;
    let f = switcher_in(&mut h, RISTRETTO_OVERLAY).await;
    assert!(
        !f.contains(DOMUX_OVERLAY),
        "no overlay row keeps the domux ground:\n{f}"
    );
}

#[tokio::test]
async fn a_theme_file_is_drawn_after_config_reload() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = switcher_in(&mut h, DOMUX_OVERLAY).await;
    assert!(!f.contains(RISTRETTO_OVERLAY), "{f}");
    h.write_theme("ristretto", RISTRETTO);
    name_theme(&h, "ristretto");
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(r["warnings"], json!([]), "{r}");
    let f = switcher_in(&mut h, RISTRETTO_OVERLAY).await;
    assert!(!f.contains(DOMUX_OVERLAY), "{f}");
}

#[tokio::test]
async fn a_theme_file_with_a_syntax_error_warns_at_start_and_auto_applies() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.write_theme("ristretto", BROKEN);
    name_theme(&h, "ristretto");
    // The warnings a start reports are the ones `load_config` answers: the server takes its
    // config from there and logs each one.
    let loaded = load_config(&h.config_path());
    assert!(loaded.error.is_none(), "a theme never blocks the config");
    let warning = loaded
        .warnings
        .iter()
        .find(|w| w.starts_with("themes/ristretto.toml line 2"))
        .unwrap_or_else(|| panic!("no warning names the file: {:?}", loaded.warnings));
    assert!(warning.ends_with("; the theme is not used"), "{warning}");
    h.restart().await;
    // Auto on a client that says nothing of its desktop is domux.
    let f = switcher_in(&mut h, DOMUX_OVERLAY).await;
    assert!(!f.contains(RISTRETTO_OVERLAY), "{f}");
}

#[tokio::test]
async fn a_theme_file_that_breaks_on_reload_keeps_the_theme_drawn_before() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.write_theme("ristretto", RISTRETTO);
    name_theme(&h, "ristretto");
    h.api("config.reload", json!({})).await.unwrap();
    switcher_in(&mut h, RISTRETTO_OVERLAY).await;

    // The theme breaks while the rest of the config changes: the rest applies, the theme
    // drawn before stays.
    h.write_theme("ristretto", BROKEN);
    std::fs::write(
        h.config_path(),
        "[keys]\nleader = \"C-b\"\n[theme]\nname = \"ristretto\"\n",
    )
    .unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"].is_null(), "a theme never blocks the config: {r}");
    let warnings: Vec<&str> = r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("themes/ristretto.toml line 2")
                && w.ends_with("; the theme is not used")),
        "{warnings:?}"
    );
    let f = switcher_in(&mut h, RISTRETTO_OVERLAY).await;
    assert!(!f.contains(DOMUX_OVERLAY), "{f}");
    h.key(h.client.clone(), "C-b").await;
    h.wait_for(h.client.clone(), |f| f.contains("C-b  ? keys"), WAIT)
        .await;
}

/// A terminal that answered `colors`.
fn caps(colors: TerminalColors) -> Capabilities {
    Capabilities {
        truecolor: true,
        colors,
        ..Default::default()
    }
}

fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// A harness whose config names `name` as its theme.
async fn start_named(name: &str) -> Harness {
    let mut config = Config::default();
    config.theme.name = name.into();
    Harness::start(config, 80, 24).await
}

/// Where `text` starts on the screen, as `(x, y)`, reading one cell per character.
fn find(buf: &Buffer, text: &str) -> (u16, u16) {
    let want: Vec<String> = text.chars().map(|c| c.to_string()).collect();
    for y in 0..buf.area.height {
        let row: Vec<(String, u16)> = (0..buf.area.width)
            .filter_map(|x| {
                let s = buf[(x, y)].symbol();
                (!s.is_empty()).then(|| (s.to_string(), x))
            })
            .collect();
        if row.len() < want.len() {
            continue;
        }
        for i in 0..=row.len() - want.len() {
            if (0..want.len()).all(|k| row[i + k].0 == want[k]) {
                return (row[i].1, y);
            }
        }
    }
    panic!("{text:?} is not on the screen")
}

/// Opens the switcher on `client` and answers its frame and cells once it is drawn in `ground`.
async fn switcher_on(h: &mut Harness, client: &ClientId, ground: &str) -> (String, Buffer) {
    h.api("switcher.open", json!({"client": client.as_str()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            client.clone(),
            |f| f.contains("┌ Navigator") && f.contains(ground),
            WAIT,
        )
        .await;
    (f, h.buffer(client))
}

const CLAUDE_STARTS: &str = r#"{"hook_event_name":"SessionStart","session_id":"c1"}"#;
const CLAUDE_WAITS: &str = r#"{"hook_event_name":"Notification","session_id":"c1","notification_type":"permission_prompt","message":"m"}"#;
const CODEX_STARTS: &str = r#"{"hook_event_name":"SessionStart","session_id":"x1"}"#;

#[tokio::test]
async fn the_switcher_is_drawn_in_the_terminals_colours_under_the_terminal_theme() {
    let mut h = start_named("terminal").await;
    let c = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Unknown)
        .await;
    let (f, buf) = switcher_on(&mut h, &c, RISTRETTO_OVERLAY).await;
    let (x, y) = find(&buf, "┌ Navigator");
    assert_eq!(buf[(x, y)].fg, hex(0xf38d70), "the focused border:\n{f}");
    assert_eq!(buf[(x + 1, y + 1)].bg, hex(0x2c2525), "the overlay:\n{f}");
    // The workspace row inside the box, not the top bar's "proj › main".
    let (x, y) = find(&buf, "│    main");
    assert_eq!(buf[(x + 5, y)].bg, hex(0x413939), "the selected row:\n{f}");
    let (x, y) = find(&buf, "⏎ open");
    assert_eq!(buf[(x, y)].fg, hex(0x85dacc), "the footer keys:\n{f}");
    assert_eq!(buf[(0, 0)].bg, hex(0x231e1e), "the top bar:\n{f}");
}

#[tokio::test]
async fn auto_draws_the_terminal_theme_for_a_client_on_omarchy() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let c = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Omarchy)
        .await;
    let (f, buf) = switcher_on(&mut h, &c, RISTRETTO_OVERLAY).await;
    assert!(!f.contains(DOMUX_OVERLAY), "{f}");
    assert_eq!(buf[(0, 0)].bg, hex(0x231e1e), "{f}");
}

#[tokio::test]
async fn auto_draws_the_domux_theme_for_a_client_that_is_not_on_omarchy() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let c = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Unknown)
        .await;
    let (f, buf) = switcher_on(&mut h, &c, DOMUX_OVERLAY).await;
    assert!(!f.contains(RISTRETTO_OVERLAY), "{f}");
    assert_eq!(buf[(0, 0)].bg, hex(0x181825), "{f}");
}

#[tokio::test]
async fn kind_colours_keep_their_domux_values_on_ristretto_under_the_terminal_theme() {
    let mut h = start_named("terminal").await;
    let c = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Unknown)
        .await;
    let pane = h.focused_pane(c.clone());
    h.report(pane, AgentKind::Claude, CLAUDE_STARTS).await;
    let (f, buf) = switcher_on(&mut h, &c, RISTRETTO_OVERLAY).await;
    let (x, y) = find(&buf, "└ claude");
    assert_eq!(buf[(x + 2, y)].fg, hex(0xde7356), "the claude row:\n{f}");
}

#[tokio::test]
async fn a_codex_row_is_drawn_darker_on_catppuccin_latte_under_the_terminal_theme() {
    let mut h = start_named("terminal").await;
    let c = h
        .attach_with(80, 24, caps(testing::CATPPUCCIN_LATTE), Desktop::Unknown)
        .await;
    let pane = h.focused_pane(c.clone());
    h.report(pane, AgentKind::Codex, CODEX_STARTS).await;
    let (f, buf) = switcher_on(&mut h, &c, "bg=#eff1f5").await;
    let (x, y) = find(&buf, "└ codex");
    assert_eq!(buf[(x + 2, y)].fg, hex(0x6b8cc2), "the codex row:\n{f}");
}

#[tokio::test]
async fn the_accent_fill_is_one_run_under_the_terminal_theme() {
    let mut h = start_named("terminal").await;
    let c = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Unknown)
        .await;
    h.key(c.clone(), "C-a").await;
    h.key(c.clone(), ",").await;
    let f = h
        .wait_for(c.clone(), |f| f.contains("Name tab 1 ›"), WAIT)
        .await;
    let buf = h.buffer(&c);
    let filled: Vec<(u16, u16)> = (0..buf.area.height)
        .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].bg == hex(0xf38d70))
        .collect();
    assert!(!filled.is_empty(), "nothing is accent-filled:\n{f}");
    let (first, row) = filled[0];
    let (last, last_row) = filled[filled.len() - 1];
    assert_eq!(row, last_row, "the accent fill spans two rows:\n{f}");
    assert_eq!(
        filled.len(),
        usize::from(last - first + 1),
        "the accent fill is more than one run:\n{f}"
    );
}

#[tokio::test]
async fn a_colours_message_recolours_the_chrome_and_leaves_pane_cells_alone() {
    let mut h = start_named("terminal").await;
    let c = h
        .attach_with(80, 24, caps(TerminalColors::default()), Desktop::Unknown)
        .await;
    let pane = h.focused_pane(c.clone());
    h.feed_pane(pane, b"\x1b[38;2;1;2;3mhi\x1b[0m there").await;
    let f = h
        .wait_for(c.clone(), |f| f.contains("hi there"), WAIT)
        .await;
    let before = h.buffer(&c);
    assert_eq!(
        before[(0, 0)].bg,
        hex(0x181825),
        "no answers, so domux:\n{f}"
    );
    let (x, y) = find(&before, "hi there");
    let pane_cells = |buf: &Buffer| -> Vec<(Color, Color)> {
        (x..x + 8)
            .map(|x| (buf[(x, y)].fg, buf[(x, y)].bg))
            .collect()
    };
    assert_eq!(before[(x, y)].fg, hex(0x010203), "{f}");

    h.send_colors(c.clone(), testing::RISTRETTO).await;
    let f = h
        .wait_for(c.clone(), |f| f.contains("bg=#231e1e"), WAIT)
        .await;
    let after = h.buffer(&c);
    assert_eq!(after[(0, 0)].bg, hex(0x231e1e), "the top bar:\n{f}");
    assert_eq!(
        pane_cells(&after),
        pane_cells(&before),
        "the pane's own cells keep their colours:\n{f}"
    );
}

#[tokio::test]
async fn a_colours_message_is_not_the_readers_activity() {
    let mut h = start_named("terminal").await;
    let a = h
        .attach_with(80, 24, caps(TerminalColors::default()), Desktop::Unknown)
        .await;
    let b = h
        .attach_with(80, 24, caps(TerminalColors::default()), Desktop::Unknown)
        .await;
    assert_eq!(h.model().most_recent_client(), Some(b.clone()));
    h.send_colors(a.clone(), testing::RISTRETTO).await;
    h.wait_for(a.clone(), |f| f.contains("bg=#231e1e"), WAIT)
        .await;
    let model = h.model();
    assert_eq!(
        model.client(&a).unwrap().caps.colors,
        testing::RISTRETTO,
        "the model holds the colours"
    );
    assert_eq!(model.most_recent_client(), Some(b));
}

#[tokio::test]
async fn two_clients_on_different_terminals_each_get_their_own_chrome() {
    let mut h = start_named("terminal").await;
    let brown = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Unknown)
        .await;
    let light = h
        .attach_with(80, 24, caps(testing::CATPPUCCIN_LATTE), Desktop::Unknown)
        .await;
    let (f, buf) = switcher_on(&mut h, &brown, RISTRETTO_OVERLAY).await;
    assert_eq!(buf[(0, 0)].bg, hex(0x231e1e), "{f}");
    let (f, buf) = switcher_on(&mut h, &light, "bg=#eff1f5").await;
    assert!(!f.contains(RISTRETTO_OVERLAY), "{f}");
    let (x, y) = find(&buf, "┌ Navigator");
    assert_eq!(buf[(x + 1, y + 1)].bg, hex(0xeff1f5), "{f}");
    // The first client still has its own.
    let f = h.frame(brown.clone()).await;
    assert!(
        f.contains(RISTRETTO_OVERLAY) && !f.contains("bg=#eff1f5"),
        "{f}"
    );
}

#[tokio::test]
async fn a_terminal_that_did_not_answer_is_drawn_in_the_domux_theme_under_terminal() {
    let mut h = start_named("terminal").await;
    let c = h
        .attach_with(80, 24, caps(TerminalColors::default()), Desktop::Omarchy)
        .await;
    let (f, buf) = switcher_on(&mut h, &c, DOMUX_OVERLAY).await;
    assert_eq!(buf[(0, 0)].bg, hex(0x181825), "{f}");
    let (x, y) = find(&buf, "┌ Navigator");
    assert_eq!(buf[(x, y)].fg, hex(0xcba6f7), "{f}");
}

#[tokio::test]
async fn the_waiting_dot_is_red_under_the_terminal_theme_on_vantablack() {
    let mut h = start_named("terminal").await;
    let c = h
        .attach_with(80, 24, caps(testing::VANTABLACK), Desktop::Unknown)
        .await;
    let pane = h.focused_pane(c.clone());
    h.report(pane, AgentKind::Claude, CLAUDE_WAITS).await;
    let (f, buf) = switcher_on(&mut h, &c, "bg=#000000").await;
    let (x, y) = find(&buf, "claude ◉");
    assert_eq!(
        buf[(x + 7, y)].fg,
        hex(0xf38ba8),
        "vantablack's red slot is grey, so the dot takes the domux red:\n{f}"
    );
}

#[tokio::test]
async fn a_client_is_told_to_follow_colours_only_when_its_theme_reads_the_terminal() {
    // Auto reads the terminal on Omarchy and nowhere else.
    // A client starts out not following, so only a theme that reads the terminal is said.
    let mut h = Harness::start(Config::default(), 80, 24).await;
    assert_eq!(h.follow_colors(h.client.clone()).await, Vec::<bool>::new());
    let omarchy = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Omarchy)
        .await;
    assert_eq!(h.follow_colors(omarchy).await, vec![true]);

    let mut h = start_named("domux").await;
    let omarchy = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Omarchy)
        .await;
    assert_eq!(h.follow_colors(omarchy).await, Vec::<bool>::new());

    let mut h = start_named("terminal").await;
    assert_eq!(h.follow_colors(h.client.clone()).await, vec![true]);

    // A theme file written in hex reads nothing from the terminal.
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.write_theme("ristretto", RISTRETTO);
    name_theme(&h, "ristretto");
    h.api("config.reload", json!({})).await.unwrap();
    let omarchy = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Omarchy)
        .await;
    assert_eq!(h.follow_colors(omarchy).await, Vec::<bool>::new());
}

#[tokio::test]
async fn a_config_reload_that_changes_the_theme_tells_each_client_whether_to_follow() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let elsewhere = h.client.clone();
    let omarchy = h
        .attach_with(80, 24, caps(testing::RISTRETTO), Desktop::Omarchy)
        .await;
    assert_eq!(h.follow_colors(elsewhere.clone()).await, Vec::<bool>::new());
    assert_eq!(h.follow_colors(omarchy.clone()).await, vec![true]);

    name_theme(&h, "domux");
    h.api("config.reload", json!({})).await.unwrap();
    assert_eq!(h.follow_colors(omarchy.clone()).await, vec![true, false]);
    assert_eq!(
        h.follow_colors(elsewhere.clone()).await,
        Vec::<bool>::new(),
        "a client whose answer did not change is told nothing"
    );

    name_theme(&h, "terminal");
    h.api("config.reload", json!({})).await.unwrap();
    assert_eq!(
        h.follow_colors(omarchy.clone()).await,
        vec![true, false, true]
    );
    assert_eq!(h.follow_colors(elsewhere.clone()).await, vec![true]);

    // A reload that changes nothing says nothing.
    h.api("config.reload", json!({})).await.unwrap();
    assert_eq!(h.follow_colors(omarchy).await, vec![true, false, true]);
    assert_eq!(h.follow_colors(elsewhere).await, vec![true]);
}
