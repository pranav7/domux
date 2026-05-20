package main

import (
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
