package main

import (
	"strings"
	"testing"
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
