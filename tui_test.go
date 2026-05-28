package main

import (
	"path/filepath"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

func TestViewShowsNotesForUnselectedActiveItems(t *testing.T) {
	m := model{
		width:  100,
		height: 30,
		cursor: 1,
		list: &List{
			Worktree: "/tmp/domux-test",
			Active: []Item{
				{Title: "first task", Notes: "first task notes"},
				{Title: "selected task"},
			},
		},
	}

	view := m.View()
	if !strings.Contains(view, "first task notes") {
		t.Fatalf("expected unselected task notes to be visible in view:\n%s", view)
	}
}

func TestShortenPathRecognizesCurrentAndLegacyWorktrees(t *testing.T) {
	cases := []string{
		"/tmp/project/.domux/worktrees/workspace-1",
		"/tmp/project/.baag/worktrees/workspace-1",
	}
	for _, path := range cases {
		if got := shortenPath(path); got != "project (workspace-1)" {
			t.Fatalf("shortenPath(%q) = %q", path, got)
		}
	}
}

func TestShiftArrowsMoveActiveTodo(t *testing.T) {
	m := model{
		path:   filepath.Join(t.TempDir(), "todo.md"),
		cursor: 1,
		list: &List{Active: []Item{
			{Title: "first"},
			{Title: "second"},
			{Title: "third"},
		}},
	}

	next, _ := m.handleKey(tea.KeyMsg{Type: tea.KeyShiftDown})
	pm := next.(model)
	assertActiveTitles(t, pm.list.Active, "first", "third", "second")
	if pm.cursor != 2 {
		t.Fatalf("cursor = %d, want 2", pm.cursor)
	}
	if pm.err != nil {
		t.Fatalf("shift down err: %v", pm.err)
	}

	next, _ = pm.handleKey(tea.KeyMsg{Type: tea.KeyShiftUp})
	pm = next.(model)
	assertActiveTitles(t, pm.list.Active, "first", "second", "third")
	if pm.cursor != 1 {
		t.Fatalf("cursor = %d, want 1", pm.cursor)
	}
	if pm.err != nil {
		t.Fatalf("shift up err: %v", pm.err)
	}
}

func TestWrapNoteLinesFitsInnerWidth(t *testing.T) {
	innerWidth := 32
	lines := wrapNoteLines("alpha beta gamma delta epsilon zeta", innerWidth)
	if len(lines) < 2 {
		t.Fatalf("expected wrapped note, got %q", lines)
	}

	maxWidth := innerWidth - notePadding - lipgloss.Width(notePrefix)
	for _, line := range lines {
		if lipgloss.Width(line) > maxWidth {
			t.Fatalf("line %q width %d exceeds %d", line, lipgloss.Width(line), maxWidth)
		}
		if strings.HasPrefix(line, " ") {
			t.Fatalf("wrapped line kept leading space: %q", line)
		}
	}
}

func assertActiveTitles(t *testing.T, items []Item, want ...string) {
	t.Helper()
	var got []string
	for _, item := range items {
		got = append(got, item.Title)
	}
	if strings.Join(got, "\x00") != strings.Join(want, "\x00") {
		t.Fatalf("titles = %#v, want %#v", got, want)
	}
}

func TestWrapNoteLinesBreaksLongWords(t *testing.T) {
	innerWidth := 12
	lines := wrapNoteLines("supercalifragilistic", innerWidth)
	if len(lines) < 2 {
		t.Fatalf("expected long word to wrap, got %q", lines)
	}

	maxWidth := innerWidth - notePadding - lipgloss.Width(notePrefix)
	for _, line := range lines {
		if lipgloss.Width(line) > maxWidth {
			t.Fatalf("line %q width %d exceeds %d", line, lipgloss.Width(line), maxWidth)
		}
	}
}
