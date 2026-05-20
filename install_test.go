package main

import "testing"

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
