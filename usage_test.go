package main

import (
	"context"
	"strings"
	"testing"
	"time"
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
	for _, want := range []string{"█▀▀", "CLAUDE USAGE", "CODEX USAGE", "Current session", "15% used", "Current week (Fable)", "4% used", "Unavailable"} {
		if !strings.Contains(out, want) {
			t.Fatalf("view missing %q:\n%s", want, out)
		}
	}
	if !strings.Contains(out, "━") || !strings.Contains(out, "╌") {
		t.Fatalf("view missing bar glyphs:\n%s", out)
	}
}

func TestRenderUsageSectionPadsWindows(t *testing.T) {
	out := stripANSI(renderUsageSection("CLAUDE USAGE", claudeCodeOrange, []UsageWindow{
		{Label: "Current session", Percent: 15},
		{Label: "Current week (all models)", Percent: 24},
	}, "Unavailable"))
	if !strings.Contains(out, "15% used\n\nCurrent week") {
		t.Fatalf("limits should have a blank line between them:\n%s", out)
	}
}

func TestUsageViewShowsUnavailableOnError(t *testing.T) {
	m := newUsageModel(fakeProvider{err: errNoCredentials})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{err: errNoCredentials})
	out := stripANSI(next.(usageModel).View())
	if !strings.Contains(out, "CLAUDE USAGE") || !strings.Contains(out, "CODEX USAGE") || !strings.Contains(out, "Unavailable") {
		t.Fatalf("expected unavailable state:\n%s", out)
	}
	if strings.Contains(out, "% used") {
		t.Fatalf("error view must not render fabricated bars:\n%s", out)
	}
}

func TestFormatAge(t *testing.T) {
	cases := map[time.Duration]string{
		30 * time.Second:  "just now",
		90 * time.Second:  "1m ago",
		11 * time.Minute:  "11m ago",
		2 * time.Hour:     "2h ago",
		125 * time.Minute: "2h5m ago",
	}
	for d, want := range cases {
		if got := formatAge(d); got != want {
			t.Fatalf("formatAge(%v) = %q, want %q", d, got, want)
		}
	}
}

func TestUsageViewShowsUpdatedAgeWhenStale(t *testing.T) {
	snap := UsageSnapshot{
		Windows:   []UsageWindow{{Label: "Current session", Percent: 15}},
		FetchedAt: time.Now().Add(-usageCacheTTL - time.Minute),
	}
	m := newUsageModel(fakeProvider{snap: snap})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{snapshot: snap})
	out := stripANSI(next.(usageModel).View())
	if !strings.Contains(out, "updated") || !strings.Contains(out, "ago ·") {
		t.Fatalf("expected an updated-age footer:\n%s", out)
	}
}

func TestUsageViewShowsUpdatedAgeWhenFresh(t *testing.T) {
	snap := UsageSnapshot{
		Windows:   []UsageWindow{{Label: "Current session", Percent: 15}},
		FetchedAt: time.Now().Add(-5 * time.Second),
	}
	m := newUsageModel(fakeProvider{snap: snap})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{snapshot: snap})
	out := stripANSI(next.(usageModel).View())
	if !strings.Contains(out, "updated just now ·") {
		t.Fatalf("expected the updated-age footer even for fresh data:\n%s", out)
	}
}

func TestUsageViewOmitsUpdatedAgeWithoutFetchedAt(t *testing.T) {
	m := loadedTestModel(t) // FetchedAt is zero -> nothing truthful to claim
	out := stripANSI(m.View())
	if strings.Contains(out, "updated") {
		t.Fatalf("view should not claim an age for a zero FetchedAt:\n%s", out)
	}
}

func TestUsageViewRulesOffFooter(t *testing.T) {
	m := loadedTestModel(t)
	lines := strings.Split(stripANSI(m.View()), "\n")
	// Inner content of each row, with the frame's side borders peeled off.
	inner := make([]string, len(lines))
	for i, line := range lines {
		inner[i] = strings.TrimSpace(strings.Trim(strings.TrimSpace(line), "│"))
	}
	ruleAt := -1
	for i, s := range inner {
		if s != "" && strings.Trim(s, "─") == "" && !strings.ContainsAny(lines[i], "╭╰") {
			ruleAt = i
			break
		}
	}
	if ruleAt < 0 {
		t.Fatalf("expected a horizontal rule above the footer:\n%s", stripANSI(m.View()))
	}
	if inner[ruleAt-1] != "" || inner[ruleAt+1] != "" {
		t.Fatalf("rule should be blank-line padded on both sides:\n%s", stripANSI(m.View()))
	}
	if !strings.Contains(inner[ruleAt+2], "esc close") {
		t.Fatalf("footer should follow the padded rule:\n%s", stripANSI(m.View()))
	}
	if got, want := len([]rune(inner[ruleAt])), len([]rune(inner[ruleAt+2])); got < want {
		t.Fatalf("rule (%d) should span at least the footer width (%d)", got, want)
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
		{Source: usageCodex, Label: "Current session", Percent: 60},
		{Source: usageCodex, Label: "Current week", Percent: 11},
	}}
	got := stripANSI(renderUsageIndicator(snap))
	want := "✷ session 15% · week 24% · fable 4% | ✷ session 60% · week 11%"
	if got != want {
		t.Fatalf("indicator = %q, want %q", got, want)
	}
}

func TestTmuxUsageBadgeShowsClaudeAndCodex(t *testing.T) {
	snap := UsageSnapshot{Windows: []UsageWindow{
		{Label: "Current week (all models)", Percent: 80},
		{Label: "Current session", Percent: 33},
		{Label: "Current week (Fable)", Percent: 95},
		{Source: usageCodex, Label: "Current session", Percent: 60},
		{Source: usageCodex, Label: "Current week", Percent: 11},
	}}
	got := tmuxUsageBadge(fakeProvider{snap: snap})
	want := "#[default]#[fg=#D97757,bold]✷ #[fg=#a6e3a1,bold]s:33% #[fg=#f9e2af,bold]w:80%#[default] | #[fg=#89b4fa,bold]✷ #[fg=#a6e3a1,bold]s:60% #[fg=#a6e3a1,bold]w:11%#[default]"
	if got != want {
		t.Fatalf("badge = %q, want %q", got, want)
	}
}

func TestTmuxUsageBadgeHidesUnavailableUsage(t *testing.T) {
	if got := tmuxUsageBadge(fakeProvider{err: errNoCredentials}); got != "" {
		t.Fatalf("unavailable badge = %q", got)
	}
	if got := tmuxUsageBadge(fakeProvider{snap: UsageSnapshot{Windows: []UsageWindow{
		{Label: "Current week (Fable)", Percent: 24},
	}}}); got != "" {
		t.Fatalf("badge without session/week windows = %q", got)
	}
}

func TestTmuxUsageBadgeShowsAvailableWindow(t *testing.T) {
	got := tmuxUsageBadge(fakeProvider{snap: UsageSnapshot{Windows: []UsageWindow{
		{Label: "Current week (all models)", Percent: 95},
	}}})
	want := "#[default]#[fg=#D97757,bold]✷ #[fg=#f38ba8,bold]w:95%#[default]"
	if got != want {
		t.Fatalf("badge = %q, want %q", got, want)
	}
}

func TestUsageTagMapping(t *testing.T) {
	cases := map[string]string{
		"Current session":           "session",
		"Current week (all models)": "week",
		"Current week (Fable)":      "fable",
	}
	for label, want := range cases {
		if got := stripANSI(usageTag(label)); got != want {
			t.Fatalf("usageTag(%q) = %q, want %q", label, got, want)
		}
	}
}

func TestUsageViewRendersCodexSection(t *testing.T) {
	snap := UsageSnapshot{Windows: []UsageWindow{
		{Label: "Current session", Percent: 15},
		{Source: usageCodex, Label: "Current session", Percent: 60},
		{Source: usageCodex, Label: "Current week", Percent: 11},
	}}
	m := newUsageModel(fakeProvider{snap: snap})
	m.width, m.height = 80, 24
	next, _ := m.Update(usageFetchedMsg{snapshot: snap})
	out := stripANSI(next.(usageModel).View())
	if !strings.Contains(out, "CODEX USAGE") || !strings.Contains(out, "60% used") || !strings.Contains(out, "11% used") {
		t.Fatalf("Codex section missing expected usage:\n%s", out)
	}
}
