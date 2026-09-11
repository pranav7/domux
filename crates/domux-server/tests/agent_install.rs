//! The hook installers: a preview that writes nothing, an apply that writes with a backup, a
//! second apply that changes nothing, and V1's lines removed.
//!
//! Every test here runs against a fixture copied into a temporary directory. Nothing here
//! reads or writes the author's own home directory.

use domux_core::model::agent::AgentKind;
use domux_server::agents::hooks::EVENTS_OPENCODE;
use domux_server::agents::install::{
    apply, backup_path, hook_command, is_v1_line, is_v2_line, opencode_plugin, plan, preview,
};
use domux_server::agents::manifests::Registry;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A temporary home directory, with one fixture copied to a path inside it. The `TempDir` is
/// returned so the caller keeps it alive for the length of the test.
fn home_with(fixture: Option<(&str, &str)>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    if let Some((name, rel)) = fixture {
        let dest = dir.path().join(rel);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/settings")
                .join(name),
            &dest,
        )
        .unwrap();
    }
    let home = dir.path().to_path_buf();
    (dir, home)
}

/// Deliberately the literal path, not a name built from `names::BIN_NAME`: this is what the
/// author reads in the file after an install, and the cut-over's rename should fail it once so
/// someone reads the line it writes.
fn bin() -> PathBuf {
    PathBuf::from("/Users/pranav/bin/domux")
}

/// Every command installed for one event, in file order.
fn commands(settings: &Value, event: &str) -> Vec<String> {
    settings["hooks"][event]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .flat_map(|e| e["hooks"].as_array().cloned().unwrap_or_default())
                .filter_map(|h| h["command"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The `matcher` of the first entry of one event, absent when the entry carries none.
fn matcher(settings: &Value, event: &str) -> Option<String> {
    settings["hooks"][event][0]["matcher"]
        .as_str()
        .map(String::from)
}

#[test]
fn a_preview_writes_nothing_and_shows_what_would_change() {
    let (_d, home) = home_with(Some(("v1_claude.json", ".claude/settings.json")));
    let path = home.join(".claude/settings.json");
    let before = std::fs::read_to_string(&path).unwrap();
    let p = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap();
    let text = preview(&p);
    assert!(
        text.starts_with(&format!("Would patch {}", path.display())),
        "{text}"
    );
    assert!(
        text.contains("+ SessionStart       /Users/pranav/bin/domux agent report --agent claude"),
        "{text}"
    );
    assert!(
        text.contains("- SessionStart       domux workspace occupied"),
        "{text}"
    );
    assert!(
        text.contains("- Stop               domux ai-state clear"),
        "{text}"
    );
    assert!(
        text.contains("- statusLine         domux claude-statusline"),
        "{text}"
    );
    assert!(
        text.contains("Run it again with --apply to write it."),
        "{text}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "a preview writes nothing"
    );
}

#[test]
fn apply_writes_the_nine_events_backs_the_file_up_and_keeps_every_other_key() {
    let (_d, home) = home_with(Some(("v1_claude.json", ".claude/settings.json")));
    let path = home.join(".claude/settings.json");
    let start = std::fs::read_to_string(&path).unwrap();
    // The removals below only mean something if the fixture carries the lines they name.
    assert!(
        start.contains("domux ai-state clear"),
        "the fixture carries V1's lines"
    );
    assert!(
        start.contains(".tmux-claude-"),
        "the fixture carries V1's tmux bridge"
    );
    assert!(
        start.contains("claude-statusline"),
        "the fixture carries V1's status line"
    );

    let p = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap();
    let backup = apply(&p).unwrap();
    assert!(
        backup.to_string_lossy().contains(".domux-backup-"),
        "{backup:?}"
    );
    assert!(backup.exists());
    let after = read_json(&path);
    assert_eq!(after["theme"], "dark", "an unrelated key survived");
    for event in [
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Notification",
        "PreCompact",
        "PostCompact",
        "Stop",
    ] {
        assert!(
            commands(&after, event)
                .iter()
                .any(|c| c == &hook_command(&bin(), AgentKind::Claude)),
            "{event} is missing the domux hook"
        );
    }
    assert_eq!(
        matcher(&after, "PreToolUse").as_deref(),
        Some("*"),
        "a tool event carries the matcher V1 writes"
    );
    assert!(
        !commands(&after, "Stop")
            .iter()
            .any(|c| c.contains("ai-state")),
        "V1's line went"
    );
    assert!(
        commands(&after, "Stop")
            .iter()
            .any(|c| c.contains("afplay")),
        "the author's own line stayed"
    );
    assert!(
        !commands(&after, "Notification")
            .iter()
            .any(|c| c.contains(".tmux-claude-")),
        "V1's tmux dotfile bridge went too"
    );
    assert!(
        commands(&after, "Notification")
            .iter()
            .any(|c| c.contains("Submarine.aiff")),
        "the author's own notification line stayed"
    );
    assert!(
        after.get("statusLine").is_none(),
        "V1's statusline went with V1"
    );
}

#[test]
fn apply_twice_changes_nothing_the_second_time() {
    let (_d, home) = home_with(Some(("v1_claude.json", ".claude/settings.json")));
    let path = home.join(".claude/settings.json");
    let r = Registry::builtin();
    let first = apply(&plan(&r, AgentKind::Claude, &home, &bin()).unwrap()).unwrap();
    assert_ne!(first, path, "the first apply backed the file up");
    let once = std::fs::read_to_string(&path).unwrap();

    let second = plan(&r, AgentKind::Claude, &home, &bin()).unwrap();
    assert!(
        second.added.is_empty() && second.removed.is_empty(),
        "nothing left to do: {second:?}"
    );
    assert!(
        preview(&second).contains("Nothing to change"),
        "{}",
        preview(&second)
    );
    let backup = apply(&second).unwrap();
    assert_eq!(
        backup, path,
        "a second apply writes nothing, so it makes no backup"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
}

#[test]
fn a_missing_settings_file_is_created_with_only_the_hooks() {
    let (_d, home) = home_with(None);
    let p = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap();
    assert!(
        preview(&p).starts_with(&format!("Would create {}", p.path.display())),
        "a file that is not there is created, not patched:\n{}",
        preview(&p)
    );
    apply(&p).unwrap();
    let after = read_json(&home.join(".claude/settings.json"));
    assert_eq!(after.as_object().unwrap().len(), 1, "only hooks: {after}");
    assert_eq!(
        commands(&after, "Stop"),
        vec![hook_command(&bin(), AgentKind::Claude)]
    );
}

#[test]
fn an_empty_settings_file_gets_the_hooks_and_is_backed_up() {
    let (_d, home) = home_with(Some(("clean_claude.json", ".claude/settings.json")));
    let p = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap();
    let backup = apply(&p).unwrap();
    assert!(
        backup.to_string_lossy().contains(".domux-backup-"),
        "a file that was there is backed up even when it holds nothing: {backup:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        "{}\n",
        "and the backup holds what the file held"
    );
    let after = read_json(&home.join(".claude/settings.json"));
    assert_eq!(
        commands(&after, "Stop"),
        vec![hook_command(&bin(), AgentKind::Claude)]
    );
}

#[test]
fn a_statusline_the_author_wrote_is_left_alone() {
    let (_d, home) = home_with(Some(("custom_statusline.json", ".claude/settings.json")));
    let p = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap();
    apply(&p).unwrap();
    let after = read_json(&home.join(".claude/settings.json"));
    assert_eq!(
        after["statusLine"]["command"], "~/bin/my-status",
        "only V1's own statusline is removed"
    );
}

#[test]
fn a_settings_file_that_is_not_json_refuses_and_names_the_file_and_the_line() {
    let (_d, home) = home_with(None);
    let path = home.join(".claude/settings.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    let err = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap_err();
    assert!(
        err.to_string()
            .starts_with(&format!("cannot read {}: ", path.display())),
        "{err}"
    );
    assert!(err.to_string().contains("line 1"), "{err}");
}

#[test]
fn a_hooks_value_that_is_not_an_object_refuses_and_names_the_file() {
    let (_d, home) = home_with(None);
    let path = home.join(".claude/settings.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"hooks": []}"#).unwrap();
    let err = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap_err();
    assert!(
        err.to_string()
            .starts_with(&format!("cannot read {}: ", path.display())),
        "{err}"
    );
    assert!(err.to_string().contains("hooks"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        r#"{"hooks": []}"#,
        "a refusal leaves the file where it was"
    );
}

#[test]
fn codex_gets_its_seven_events_and_loses_v1s_five() {
    let (_d, home) = home_with(Some(("v1_codex.json", ".codex/hooks.json")));
    let path = home.join(".codex/hooks.json");
    let start = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        start.matches("ai-state").count(),
        5,
        "the fixture carries V1's five lines"
    );
    let p = plan(&Registry::builtin(), AgentKind::Codex, &home, &bin()).unwrap();
    apply(&p).unwrap();
    let after = read_json(&path);
    for event in [
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
        "Stop",
    ] {
        assert!(
            commands(&after, event)
                .iter()
                .any(|c| c == &hook_command(&bin(), AgentKind::Codex)),
            "{event}"
        );
    }
    for event in ["PreToolUse", "PostToolUse", "PermissionRequest"] {
        assert_eq!(
            matcher(&after, event).as_deref(),
            Some("*"),
            "{event} carries the matcher V1 writes"
        );
    }
    assert!(
        !serde_json::to_string(&after).unwrap().contains("ai-state"),
        "every V1 line went: {after}"
    );
}

#[test]
fn opencode_writes_a_plugin_that_posts_the_payload_domux_reads() {
    let (_d, home) = home_with(None);
    let p = plan(&Registry::builtin(), AgentKind::Opencode, &home, &bin()).unwrap();
    apply(&p).unwrap();
    let js = std::fs::read_to_string(home.join(".config/opencode/plugins/domux.js")).unwrap();
    assert!(js.contains("/Users/pranav/bin/domux"), "{js}");
    assert!(
        js.contains(r#""agent", "report", "--agent", "opencode""#),
        "{js}"
    );
    assert!(
        js.contains("hook_event_name"),
        "the plugin builds the payload domux parses:\n{js}"
    );
    assert!(
        js.contains("session_id") && js.contains("cwd"),
        "the payload carries the fields the adapter reads:\n{js}"
    );
    // Every OpenCode event `agents::hooks::parse_opencode` maps to a domux event. The list is
    // the one the adapter's fixture loop walks too, so a name in it that the plugin or the
    // fixtures do not carry fails a test. A name in neither the list nor a test is still
    // possible: nothing here reads the plugin's own switch.
    for event in EVENTS_OPENCODE {
        assert!(js.contains(event), "{event} is missing from the plugin");
    }
    // The adapter reads `message` as a string and answers absent for anything else, so the
    // plugin sends text rather than whatever the event carried.
    assert!(
        js.contains("message: messageText(input?.message)"),
        "the payload's message goes through the helper:\n{js}"
    );
    assert!(
        js.contains(r#"typeof message === "string""#) && js.contains("JSON.stringify(message)"),
        "the helper passes a string through and turns anything else into text:\n{js}"
    );
}

#[test]
fn a_plugin_that_is_already_installed_is_left_alone() {
    let (_d, home) = home_with(None);
    let path = home.join(".config/opencode/plugins/domux.js");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, opencode_plugin(&bin())).unwrap();
    let p = plan(&Registry::builtin(), AgentKind::Opencode, &home, &bin()).unwrap();
    assert!(p.added.is_empty(), "nothing left to do: {p:?}");
    assert!(preview(&p).contains("Nothing to change"), "{}", preview(&p));
    assert_eq!(apply(&p).unwrap(), path, "so it makes no backup");
}

#[test]
fn the_backup_keeps_the_previous_file_byte_for_byte() {
    let (_d, home) = home_with(Some(("v1_claude.json", ".claude/settings.json")));
    let before = std::fs::read_to_string(home.join(".claude/settings.json")).unwrap();
    let p = plan(&Registry::builtin(), AgentKind::Claude, &home, &bin()).unwrap();
    let backup = apply(&p).unwrap();
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), before);
}

#[test]
fn the_backup_name_is_the_file_and_the_stamp() {
    assert_eq!(
        backup_path(Path::new("/home/a/.claude/settings.json"), "20260905131415"),
        PathBuf::from("/home/a/.claude/settings.json.domux-backup-20260905131415"),
        "the whole file name is kept, so the backup sorts next to it"
    );
}

#[test]
fn a_binary_path_that_needs_quoting_is_quoted_for_the_shell() {
    assert_eq!(
        hook_command(Path::new("/Users/a/my bin/domux"), AgentKind::Claude),
        "'/Users/a/my bin/domux' agent report --agent claude"
    );
    assert_eq!(
        hook_command(Path::new("/Users/a/bin/domux"), AgentKind::Codex),
        "/Users/a/bin/domux agent report --agent codex",
        "a path that needs no quoting is written bare"
    );
}

#[test]
fn v1s_lines_are_recognised_and_the_authors_own_lines_are_not() {
    for command in [
        "domux ai-state CLAUDING",
        "domux ai-state clear",
        "domux workspace free && domux ai-state clear",
        "domux workspace occupied",
        "\"$HOME/bin/domux\" ai-state --agent codex CODEXING",
        "echo WAITING > ~/.tmux-claude-$(tmux display-message -p '#S')",
    ] {
        assert!(is_v1_line(command), "{command}");
        assert!(!is_v2_line(command), "{command}");
    }
    for command in [
        "afplay /System/Library/Sounds/Glass.aiff",
        "~/bin/my-status",
        "make test",
    ] {
        assert!(!is_v1_line(command), "{command}");
        assert!(!is_v2_line(command), "{command}");
    }
    // What a previous install wrote, under either binary name: the cut-over renames the
    // binary, and doctor still has to recognise the line the old name wrote.
    for command in [
        "/Users/pranav/bin/domux agent report --agent claude",
        "/Users/pranav/bin/domux agent report --agent codex",
        "'/Users/a/my bin/domux' agent report --agent opencode",
    ] {
        assert!(is_v2_line(command), "{command}");
        assert!(!is_v1_line(command), "{command}");
    }
}
