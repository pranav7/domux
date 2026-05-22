package main

import (
	"slices"
	"strings"
	"testing"
)

func TestPlanBootstrapAllPresent(t *testing.T) {
	steps := planBootstrap(bootstrapEnv{HasBrew: true, HasTmux: true, HasClaude: true, HasCodex: true})
	want := []string{
		"write ~/.config/domux/domux.tmux",
		"patch ~/.claude/settings.json (Claude Code detected)",
		"patch ~/.codex/hooks.json (Codex detected)",
		"register caffeinate (partial — no sudo)",
	}
	if got := stepLabels(steps); !slices.Equal(got, want) {
		t.Fatalf("labels = %v, want %v", got, want)
	}
}

func TestPlanBootstrapTmuxMissingBrewPresent(t *testing.T) {
	steps := planBootstrap(bootstrapEnv{HasBrew: true})
	if !strings.HasPrefix(steps[0].Label, "brew install tmux") {
		t.Fatalf("first step should be brew install tmux, got %q", steps[0].Label)
	}
}

func TestPlanBootstrapTmuxAndBrewMissingSkipsBrewStep(t *testing.T) {
	steps := planBootstrap(bootstrapEnv{})
	for _, s := range steps {
		if strings.Contains(s.Label, "brew install tmux") {
			t.Fatalf("should not emit brew step when brew is unavailable, got %q", s.Label)
		}
	}
}

func TestPlanBootstrapSkipsAbsentAITools(t *testing.T) {
	steps := planBootstrap(bootstrapEnv{HasBrew: true, HasTmux: true})
	for _, s := range steps {
		if strings.Contains(s.Label, "settings.json") || strings.Contains(s.Label, "hooks.json") {
			t.Fatalf("should not include claude/codex steps when absent, got %q", s.Label)
		}
	}
}

func TestPlanBootstrapAlwaysIncludesCaffeinate(t *testing.T) {
	steps := planBootstrap(bootstrapEnv{HasBrew: true, HasTmux: true})
	found := false
	for _, s := range steps {
		if strings.Contains(s.Label, "caffeinate") {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("caffeinate registration should always be present in plan")
	}
}

func TestConfirmAcceptsYesVariants(t *testing.T) {
	yes := []string{"y\n", "Y\n", "yes\n", " yes \n", "YES\n"}
	for _, input := range yes {
		if !confirm(strings.NewReader(input), "") {
			t.Errorf("confirm(%q) should be true", input)
		}
	}
}

func TestConfirmRejectsEverythingElse(t *testing.T) {
	no := []string{"\n", "n\n", "no\n", "yeah\n", "ok\n"}
	for _, input := range no {
		if confirm(strings.NewReader(input), "") {
			t.Errorf("confirm(%q) should be false", input)
		}
	}
}

func stepLabels(steps []bootstrapStep) []string {
	labels := make([]string, len(steps))
	for i, s := range steps {
		labels[i] = s.Label
	}
	return labels
}
