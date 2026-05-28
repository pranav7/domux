package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSplitTodoID(t *testing.T) {
	title, id := splitTodoID("Fix login redirect <!-- domux:id=abc123 -->")
	if title != "Fix login redirect" {
		t.Fatalf("title = %q", title)
	}
	if id != "abc123" {
		t.Fatalf("id = %q", id)
	}
}

func TestSplitTodoIDWithoutComment(t *testing.T) {
	title, id := splitTodoID("Fix login redirect")
	if title != "Fix login redirect" {
		t.Fatalf("title = %q", title)
	}
	if id != "" {
		t.Fatalf("id = %q", id)
	}
}

func TestCleanSessionName(t *testing.T) {
	got := cleanSessionName("Audrey App.Work")
	if got != "audrey-app-work" {
		t.Fatalf("cleanSessionName = %q", got)
	}
}

func TestLoadListKeepsDoneTodosActive(t *testing.T) {
	path := filepath.Join(t.TempDir(), "todo.md")
	data := `---
created: 2026-05-28
---

# TODOs

- [x] 2026-05-28 — Done task
- [ ] Open task

## Archive

- [x] 2026-05-27 — Archived task
`
	if err := os.WriteFile(path, []byte(data), 0644); err != nil {
		t.Fatalf("write todo: %v", err)
	}

	list, err := loadList(path)
	if err != nil {
		t.Fatalf("loadList: %v", err)
	}
	if len(list.Active) != 2 {
		t.Fatalf("active len = %d, want 2", len(list.Active))
	}
	if !list.Active[0].Done || list.Active[0].DoneDate != "2026-05-28" || list.Active[0].Title != "Done task" {
		t.Fatalf("active done item = %#v", list.Active[0])
	}
	if len(list.Archive) != 1 || list.Archive[0].Title != "Archived task" {
		t.Fatalf("archive = %#v", list.Archive)
	}
}

func TestGeneratedTmuxConfigUsesStateOnlyClear(t *testing.T) {
	config := generatedTmuxConfig()
	if !strings.Contains(config, "domux clear-state") {
		t.Fatalf("generated tmux config should use state-only clear")
	}
	if strings.Contains(config, "domux clear'") {
		t.Fatalf("generated tmux config should not bind plain domux clear")
	}
}

func TestGeneratedTmuxConfigUsesHomeDomuxBinary(t *testing.T) {
	config := generatedTmuxConfig()
	if !strings.Contains(config, "$HOME/bin/domux sessions") {
		t.Fatalf("generated tmux config should use home domux binary")
	}
}

func TestGeneratedTmuxConfigUsesLargeSwitcherPopup(t *testing.T) {
	config := generatedTmuxConfig()
	if !strings.Contains(config, `bind-key s display-popup -E -w 95% -h 95% "$HOME/bin/domux sessions"`) {
		t.Fatalf("generated tmux config should use large switcher popup")
	}
}
