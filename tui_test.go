package main

import (
	"strings"
	"testing"

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
