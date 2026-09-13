//! Themes chosen in `domux.toml` and written under `themes/` (design sections 3.4, 3.5 and
//! 7.2): a theme file is drawn from the start and after a reload, a broken one warns, and a
//! reload that breaks it keeps the theme drawn before.

use domux_core::config::Config;
use domux_server::load_config;
use domux_server::testing::Harness;
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
