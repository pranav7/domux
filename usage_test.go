package main

import (
	"context"
	"strings"
	"testing"
	"unicode/utf8"

	tea "github.com/charmbracelet/bubbletea"
)

func TestRenderBarWidthAndFill(t *testing.T) {
	bar := renderBar(50, 20)
	if utf8.RuneCountInString(bar) != 20 {
		t.Fatalf("bar rune count = %d, want 20 (%q)", utf8.RuneCountInString(bar), bar)
	}
	filled := strings.Count(bar, "━")
	empty := strings.Count(bar, "╌")
	if filled != 10 || empty != 10 {
		t.Fatalf("filled=%d empty=%d, want 10/10", filled, empty)
	}
}

func TestRenderBarBoundaries(t *testing.T) {
	if strings.Count(renderBar(0, 20), "━") != 0 {
		t.Fatalf("0%% should have no filled cells")
	}
	if strings.Count(renderBar(100, 20), "━") != 20 {
		t.Fatalf("100%% should fill all cells")
	}
	if strings.Count(renderBar(150, 20), "━") != 20 {
		t.Fatalf("over 100%% must clamp to full")
	}
	if utf8.RuneCountInString(renderBar(50, 0)) != 1 {
		t.Fatalf("width<1 should coerce to a 1-cell bar")
	}
}

func TestBarColorThresholds(t *testing.T) {
	if barColor(69) != green {
		t.Fatalf("69%% should be green")
	}
	if barColor(70) != yellow {
		t.Fatalf("70%% should be yellow")
	}
	if barColor(89) != yellow {
		t.Fatalf("89%% should be yellow")
	}
	if barColor(90) != red {
		t.Fatalf("90%% should be red")
	}
}

type fakeProvider struct {
	snap UsageSnapshot
	err  error
}

func (f fakeProvider) Fetch(ctx context.Context) (UsageSnapshot, error) { return f.snap, f.err }

func loadedTestModel(t *testing.T) usageModel {
	t.Helper()
	snap := UsageSnapshot{Windows: []UsageWindow{
		{Label: "Current session", Percent: 15},
		{Label: "Current week (all models)", Percent: 24},
		{Label: "Current week (Fable)", Percent: 4},
	}}
	m := newUsageModel(fakeProvider{snap: snap})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{snapshot: snap})
	return next.(usageModel)
}

func TestUsageViewRendersBars(t *testing.T) {
	m := loadedTestModel(t)
	out := stripANSI(m.View())
	for _, want := range []string{"Current session", "15% used", "Current week (Fable)", "4% used"} {
		if !strings.Contains(out, want) {
			t.Fatalf("view missing %q:\n%s", want, out)
		}
	}
	if !strings.Contains(out, "━") || !strings.Contains(out, "╌") {
		t.Fatalf("view missing bar glyphs:\n%s", out)
	}
}

func TestUsageViewShowsUnavailableOnError(t *testing.T) {
	m := newUsageModel(fakeProvider{err: errNoCredentials})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{err: errNoCredentials})
	out := stripANSI(next.(usageModel).View())
	if !strings.Contains(out, "Usage unavailable") {
		t.Fatalf("expected unavailable state:\n%s", out)
	}
	if strings.Contains(out, "% used") {
		t.Fatalf("error view must not render fabricated bars:\n%s", out)
	}
}

func TestUsageErrorReason(t *testing.T) {
	if !strings.Contains(usageErrorReason(errNoCredentials), "credentials") {
		t.Fatalf("bad reason for no creds")
	}
	if !strings.Contains(usageErrorReason(errAuthRejected), "re-login") {
		t.Fatalf("bad reason for auth rejected")
	}
	if !strings.Contains(usageErrorReason(context.DeadlineExceeded), "timeout") {
		t.Fatalf("bad reason for timeout")
	}
}

func TestUsageQuitKeys(t *testing.T) {
	m := loadedTestModel(t)
	// esc is delivered as its own key type; q arrives as runes.
	if _, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEsc}); cmd == nil {
		t.Fatalf("esc should return a quit command")
	}
	if _, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("q")}); cmd == nil {
		t.Fatalf("q should return a quit command")
	}
}

func TestUsageRefreshKeyRefetches(t *testing.T) {
	m := loadedTestModel(t)
	next, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("r")})
	if cmd == nil {
		t.Fatalf("r should return a fetch command")
	}
	if next.(usageModel).state != usageLoading {
		t.Fatalf("r should reset state to loading, got %v", next.(usageModel).state)
	}
}

func TestRenderUsageIndicatorEmpty(t *testing.T) {
	if got := renderUsageIndicator(UsageSnapshot{}); got != "" {
		t.Fatalf("empty snapshot should render nothing, got %q", got)
	}
}

func TestRenderUsageIndicatorSegments(t *testing.T) {
	snap := UsageSnapshot{Windows: []UsageWindow{
		{Label: "Current session", Percent: 15},
		{Label: "Current week (all models)", Percent: 24},
		{Label: "Current week (Fable)", Percent: 4},
	}}
	got := stripANSI(renderUsageIndicator(snap))
	want := "ses 15% · wk 24% · fab 4%"
	if got != want {
		t.Fatalf("indicator = %q, want %q", got, want)
	}
}

func TestUsageTagMapping(t *testing.T) {
	cases := map[string]string{
		"Current session":           "ses",
		"Current week (all models)": "wk",
		"Current week (Fable)":      "fab",
	}
	for label, want := range cases {
		if got := stripANSI(usageTag(label)); got != want {
			t.Fatalf("usageTag(%q) = %q, want %q", label, got, want)
		}
	}
}
