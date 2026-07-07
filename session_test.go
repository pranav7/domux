package main

import (
	"testing"
)

func TestAggregateAIStatesByWindow(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"claude:1_0": "CLAUDING",
		"claude:2_0": "WAITING",
		"codex:2_0":  "CODEXING",
		"claude:2_1": "CLAUDING", // second pane in window 2; WAITING must still win
	}}

	got := aggregateAIStatesByWindow(state)

	if got[1].Claude != "CLAUDING" {
		t.Errorf("window 1 Claude = %q, want CLAUDING", got[1].Claude)
	}
	if got[2].Claude != "WAITING" {
		t.Errorf("window 2 Claude = %q, want WAITING (WAITING outranks CLAUDING)", got[2].Claude)
	}
	if got[2].Codex != "CODEXING" {
		t.Errorf("window 2 Codex = %q, want CODEXING", got[2].Codex)
	}
	if _, ok := got[3]; ok {
		t.Errorf("window 3 should be absent, got %+v", got[3])
	}
}

func TestAggregateAIStatesByWindowNilAndEmpty(t *testing.T) {
	if got := aggregateAIStatesByWindow(nil); len(got) != 0 {
		t.Errorf("nil state = %+v, want empty map", got)
	}
	if got := aggregateAIStatesByWindow(&SessionState{}); len(got) != 0 {
		t.Errorf("empty state = %+v, want empty map", got)
	}
}
