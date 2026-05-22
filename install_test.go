package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPatchedClaudeSettingsAddsCompactHooks(t *testing.T) {
	path := filepath.Join(t.TempDir(), "settings.json")
	settings, err := patchedClaudeSettings(path)
	if err != nil {
		t.Fatalf("patchedClaudeSettings: %v", err)
	}
	events, _ := settings["hooks"].(map[string]any)
	preEntries, _ := events["PreCompact"].([]any)
	if !hookCommandExists(preEntries, "domux ai-state --agent claude COMPACTING") {
		t.Fatalf("PreCompact hook missing — events: %#v", events)
	}
	postEntries, _ := events["PostCompact"].([]any)
	if !hookCommandExists(postEntries, "domux ai-state --agent claude CLAUDING") {
		t.Fatalf("PostCompact hook missing — events: %#v", events)
	}
}

func TestPatchedClaudeSettingsSetsStatusLineWhenAbsent(t *testing.T) {
	path := filepath.Join(t.TempDir(), "settings.json")
	settings, err := patchedClaudeSettings(path)
	if err != nil {
		t.Fatalf("patchedClaudeSettings: %v", err)
	}
	sl, _ := settings["statusLine"].(map[string]any)
	if sl["command"] != "domux claude-statusline" {
		t.Fatalf("expected domux statusLine when absent, got %#v", sl)
	}
}

func TestPatchedClaudeSettingsPreservesExistingStatusLine(t *testing.T) {
	path := filepath.Join(t.TempDir(), "settings.json")
	existing := `{
  "statusLine": {
    "type": "command",
    "command": "bash ~/.claude/statusline-command.sh"
  }
}`
	if err := os.WriteFile(path, []byte(existing), 0600); err != nil {
		t.Fatalf("seed settings.json: %v", err)
	}
	settings, err := patchedClaudeSettings(path)
	if err != nil {
		t.Fatalf("patchedClaudeSettings: %v", err)
	}
	sl, _ := settings["statusLine"].(map[string]any)
	if sl["command"] != "bash ~/.claude/statusline-command.sh" {
		t.Fatalf("custom statusLine was clobbered: %#v", sl)
	}
	// Hooks should still be added even when statusLine is preserved.
	events, _ := settings["hooks"].(map[string]any)
	preEntries, _ := events["PreCompact"].([]any)
	if !hookCommandExists(preEntries, "domux ai-state --agent claude COMPACTING") {
		t.Fatalf("PreCompact hook should still be added when statusLine is preserved")
	}
}

func TestPatchedCodexHooksUsesCodexEvents(t *testing.T) {
	hooks := patchedCodexHooks(map[string]any{})
	events, _ := hooks["hooks"].(map[string]any)

	for _, event := range []string{"UserPromptSubmit", "PreToolUse", "PermissionRequest", "Stop"} {
		if _, ok := events[event]; !ok {
			t.Fatalf("missing %s hook", event)
		}
	}
	if _, ok := events["SessionStart"]; ok {
		t.Fatalf("Codex hooks should not include SessionStart")
	}
	if _, ok := events["SessionEnd"]; ok {
		t.Fatalf("Codex hooks should not include SessionEnd")
	}
}

func TestPatchedCodexHooksMarksCodexState(t *testing.T) {
	hooks := patchedCodexHooks(map[string]any{})
	events, _ := hooks["hooks"].(map[string]any)

	if !hookCommandExists(events["PreToolUse"].([]any), codexDomuxHook("ai-state --agent codex CODEXING")) {
		t.Fatalf("PreToolUse should mark CODEXING")
	}
	if !hookCommandExists(events["PermissionRequest"].([]any), codexDomuxHook("ai-state --agent codex WAITING")) {
		t.Fatalf("PermissionRequest should mark Codex waiting")
	}
	if !hookCommandExists(events["Stop"].([]any), codexDomuxHook("ai-state --agent codex --all clear")) {
		t.Fatalf("Stop should clear Codex state")
	}
}

func TestCodexDomuxHookUsesAbsolutePath(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)

	got := codexDomuxHook("ai-state --agent codex CODEXING")
	want := filepath.Join(homeDir, "bin", "domux") + " ai-state --agent codex CODEXING"
	if got != want {
		t.Fatalf("hook = %q, want %q", got, want)
	}
}

func TestPatchedCodexHooksPrunesOldWrappedDomuxHooks(t *testing.T) {
	oldCommand := `PATH="$HOME/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin" "$HOME/bin/domux" ai-state --agent codex CODEXING`
	hooks := patchedCodexHooks(map[string]any{
		"hooks": map[string]any{
			"PostToolUse": []any{
				map[string]any{
					"matcher": "*",
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": oldCommand,
						},
					},
				},
			},
		},
	})
	events, _ := hooks["hooks"].(map[string]any)

	if hookCommandExists(events["PostToolUse"].([]any), oldCommand) {
		t.Fatalf("old wrapped Codex hook should be pruned")
	}
	if !hookCommandExists(events["PostToolUse"].([]any), codexDomuxHook("ai-state --agent codex CODEXING")) {
		t.Fatalf("new absolute Codex hook should be added")
	}
}

func TestShellCommandPathQuotesSpecialChars(t *testing.T) {
	got := shellCommandPath("/tmp/a path/it's/domux")
	if !strings.HasPrefix(got, "'") || !strings.HasSuffix(got, "'") {
		t.Fatalf("path should be quoted: %q", got)
	}
	if !strings.Contains(got, "'\\''") {
		t.Fatalf("single quote should be escaped: %q", got)
	}
}

func TestPatchedCodexHooksPrunesCopiedClaudeDomuxHooks(t *testing.T) {
	hooks := patchedCodexHooks(map[string]any{
		"hooks": map[string]any{
			"PreToolUse": []any{
				map[string]any{
					"matcher": "*",
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "echo CLAUDING > ~/.tmux-claude-$(tmux display-message -p '#S' 2>/dev/null || echo default)",
						},
					},
				},
			},
			"SessionStart": []any{
				map[string]any{
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "echo occupied > ~/.tmux-workspace-$(tmux display-message -p '#S' 2>/dev/null || echo default)",
						},
					},
				},
			},
			"Stop": []any{
				map[string]any{
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "echo IDLE > ~/.tmux-claude-$(tmux display-message -p '#S' 2>/dev/null || echo default)",
						},
						map[string]any{
							"type":    "command",
							"command": "afplay /System/Library/Sounds/Glass.aiff",
						},
					},
				},
			},
		},
	})
	events, _ := hooks["hooks"].(map[string]any)

	if hookCommandExists(events["PreToolUse"].([]any), "echo CLAUDING > ~/.tmux-claude-$(tmux display-message -p '#S' 2>/dev/null || echo default)") {
		t.Fatalf("copied Claude PreToolUse hook should be pruned")
	}
	if hookCommandExists(events["Stop"].([]any), "echo IDLE > ~/.tmux-claude-$(tmux display-message -p '#S' 2>/dev/null || echo default)") {
		t.Fatalf("copied Claude Stop hook should be pruned")
	}
	if _, ok := events["SessionStart"]; ok {
		t.Fatalf("copied workspace SessionStart hook should be pruned")
	}
	if !hookCommandExists(events["Stop"].([]any), "afplay /System/Library/Sounds/Glass.aiff") {
		t.Fatalf("custom Stop hook should remain")
	}
}

func TestPatchedCodexHooksPrunesOldDomuxSessionStart(t *testing.T) {
	hooks := patchedCodexHooks(map[string]any{
		"hooks": map[string]any{
			"SessionStart": []any{
				map[string]any{
					"matcher": "startup|resume|clear",
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "domux workspace occupied",
						},
					},
				},
			},
			"Stop": []any{
				map[string]any{
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "domux ai-state --agent codex clear",
						},
					},
				},
			},
		},
	})
	events, _ := hooks["hooks"].(map[string]any)

	if _, ok := events["SessionStart"]; ok {
		t.Fatalf("old domux SessionStart hook should be pruned")
	}
	if hookCommandExists(events["Stop"].([]any), "domux ai-state --agent codex clear") {
		t.Fatalf("old pane-only Codex Stop hook should be pruned")
	}
	if !hookCommandExists(events["Stop"].([]any), codexDomuxHook("ai-state --agent codex --all clear")) {
		t.Fatalf("new all-panes Codex Stop hook should be added")
	}
}
