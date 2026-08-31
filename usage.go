package main

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

const usageBarWidth = 20

// fableCrimson highlights the word "Fable" wherever it appears. This is the
// same muted, light-brick red the statusline uses for the Fable/Sonnet model
// name (~/dotfiles/claude/statusline-command.sh), distinct from the palette's
// pinkish `red` used for >=90% bar pressure.
var fableCrimson = lipgloss.Color("#C2797A")

// claudeCodeOrange is Claude Code's brand terracotta.
var claudeCodeOrange = lipgloss.Color("#D97757")

// renderBar returns a plain (no ANSI) meter: `━` for filled cells, `╌` for
// empty, matching the statusline meter style. Coloring is applied by the caller.
func renderBar(percent, width int) string {
	if width < 1 {
		width = 1
	}
	if percent < 0 {
		percent = 0
	}
	if percent > 100 {
		percent = 100
	}
	filled := percent * width / 100
	if filled > width {
		filled = width
	}
	var b strings.Builder
	for i := 0; i < filled; i++ {
		b.WriteString("━")
	}
	for i := 0; i < width-filled; i++ {
		b.WriteString("╌")
	}
	return b.String()
}

// barColor maps usage pressure to the green->amber->red thresholds used by the
// statusline (green <70, yellow 70-89, red >=90).
func barColor(percent int) lipgloss.Color {
	switch {
	case percent >= 90:
		return red
	case percent >= 70:
		return yellow
	default:
		return green
	}
}

// --- bubbletea model ---

type usageState int

const (
	usageLoading usageState = iota
	usageLoaded
	usageErr
)

type usageModel struct {
	provider UsageProvider
	state    usageState
	snapshot UsageSnapshot
	err      error
	width    int
	height   int
}

type usageFetchedMsg struct {
	snapshot UsageSnapshot
	err      error
}

func newUsageModel(p UsageProvider) usageModel {
	return usageModel{provider: p, state: usageLoading}
}

func (m usageModel) Init() tea.Cmd {
	return usageFetchCmd(m.provider)
}

func usageFetchCmd(p UsageProvider) tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), usageFetchTimeout)
		defer cancel()
		snap, err := p.Fetch(ctx)
		return usageFetchedMsg{snapshot: snap, err: err}
	}
}

func (m usageModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
		return m, nil
	case usageFetchedMsg:
		if msg.err != nil {
			m.state, m.err = usageErr, msg.err
		} else {
			m.state, m.snapshot = usageLoaded, msg.snapshot
		}
		return m, nil
	case tea.KeyMsg:
		switch msg.String() {
		case "esc", "q", "ctrl+c":
			return m, tea.Quit
		case "r":
			m.state = usageLoading
			return m, usageFetchCmd(m.provider)
		}
	}
	return m, nil
}

// formatAge renders a duration as a short "X ago" string for the popup
// footer's "updated ... ago" line, so freshness is visible at a glance
// whether the snapshot is a live fetch or a rate-limit fallback.
func formatAge(d time.Duration) string {
	if d < time.Minute {
		return "just now"
	}
	if d < time.Hour {
		return fmt.Sprintf("%dm ago", int(d.Minutes()))
	}
	h := int(d.Hours())
	m := int(d.Minutes()) % 60
	if m == 0 {
		return fmt.Sprintf("%dh ago", h)
	}
	return fmt.Sprintf("%dh%dm ago", h, m)
}

func usageErrorReason(err error) string {
	switch {
	case errors.Is(err, errNoCredentials):
		return "no credentials found — log in with Claude"
	case errors.Is(err, errAuthRejected):
		return "auth rejected — re-login in Claude"
	case errors.Is(err, errRateLimited):
		return "rate-limited — try again in a moment"
	case errors.Is(err, context.DeadlineExceeded):
		return "network timeout"
	default:
		return "unexpected response"
	}
}

var (
	uTitle   = lipgloss.NewStyle().Foreground(subtext0)
	uLabel   = lipgloss.NewStyle().Foreground(subtext0)
	uPercent = lipgloss.NewStyle().Foreground(text).Bold(true)
	uFable   = lipgloss.NewStyle().Foreground(fableCrimson).Bold(true)
	uReset   = lipgloss.NewStyle().Foreground(overlay0)
	uFooter  = lipgloss.NewStyle().Foreground(overlay0)
	uRule    = lipgloss.NewStyle().Foreground(surface1)
	// uFrame is a compact bordered modal with generous inner padding; it hugs
	// its content rather than filling the whole tmux popup.
	uFrame = lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(claudeCodeOrange).
		Padding(1, 3)
)

// blockGlyphs is the compact mural used for the Claude Code usage modal.
var blockGlyphs = map[rune][]string{
	'C': {"█▀▀", "█  ", "█▄▄"},
	'L': {"█  ", "█  ", "█▄▄"},
	'A': {"█▀█", "█▀█", "█ █"},
	'U': {"█ █", "█ █", "█▄█"},
	'D': {"█▀▄", "█ █", "█▄▀"},
	'E': {"█▀▀", "█▀ ", "█▄▄"},
	'O': {"█▀█", "█ █", "█▄█"},
	'X': {"█ █", " █ ", "█ █"},
}

const blockGlyphRows = 3

func renderBlockWord(word string) []string {
	rows := make([]string, blockGlyphRows)
	for _, ch := range word {
		g := blockGlyphs[ch]
		for r := 0; r < blockGlyphRows; r++ {
			if rows[r] != "" {
				rows[r] += " "
			}
			rows[r] += g[r]
		}
	}
	return rows
}

func renderClaudeCodeLogo() string {
	logo := lipgloss.NewStyle().Foreground(claudeCodeOrange).Bold(true)
	rows := append(renderBlockWord("CLAUDE"), "")
	rows = append(rows, renderBlockWord("CODE")...)
	var b strings.Builder
	for i, line := range rows {
		b.WriteString(logo.Render(line))
		if i == len(rows)-1 {
			b.WriteString("  " + uTitle.Render("usage"))
		}
		b.WriteString("\n")
	}
	return b.String()
}

func renderCodexLogo() string {
	logo := lipgloss.NewStyle().Foreground(blue).Bold(true)
	rows := renderBlockWord("CODEX")
	var b strings.Builder
	for i, line := range rows {
		b.WriteString(logo.Render(line))
		if i == len(rows)-1 {
			b.WriteString("  " + uTitle.Render("usage"))
		}
		b.WriteString("\n")
	}
	return b.String()
}

func (m usageModel) View() string {
	if m.width == 0 {
		return ""
	}
	var b strings.Builder
	b.WriteString(renderClaudeCodeLogo())
	b.WriteString("\n")
	switch m.state {
	case usageLoading:
		b.WriteString(renderUsageSection(nil, "Fetching usage…"))
		b.WriteString("\n\n")
		b.WriteString(renderCodexLogo())
		b.WriteString("\n")
		b.WriteString(renderUsageSection(nil, "Fetching usage…"))
	case usageErr:
		reason := "Unavailable — " + usageErrorReason(m.err)
		b.WriteString(renderUsageSection(nil, reason))
		b.WriteString("\n\n")
		b.WriteString(renderCodexLogo())
		b.WriteString("\n")
		b.WriteString(renderUsageSection(nil, reason))
	case usageLoaded:
		b.WriteString(renderUsageSection(usageWindows(m.snapshot, usageClaude), "Unavailable"))
		b.WriteString("\n\n")
		b.WriteString(renderCodexLogo())
		b.WriteString("\n")
		b.WriteString(renderUsageSection(usageWindows(m.snapshot, usageCodex), "Unavailable"))
	}
	footer := "r refresh · esc close"
	if m.state == usageLoaded && !m.snapshot.FetchedAt.IsZero() {
		footer = "updated " + formatAge(time.Since(m.snapshot.FetchedAt)) + " · " + footer
	}
	// Separate the footer from the usage sections with a rule spanning the
	// content width, blank-line padded on both sides so it reads as a divider
	// rather than crowding the last "Resets …" line.
	body := b.String()
	ruleWidth := lipgloss.Width(body)
	if w := lipgloss.Width(footer); w > ruleWidth {
		ruleWidth = w
	}
	rule := uRule.Render(strings.Repeat("─", ruleWidth))
	content := body + "\n\n" + rule + "\n\n" + uFooter.Render(footer)
	// lipgloss.Place doesn't clip oversized content — it just overflows the
	// requested size, which for a fixed-size tmux popup means the frame's
	// top rows scroll out of the popup and the pane behind it shows through.
	// Clamp defensively so the modal can never exceed the popup it's drawn
	// into, even if a future window/label makes the content taller/wider.
	frame := lipgloss.NewStyle().MaxWidth(m.width).MaxHeight(m.height).Render(uFrame.Render(content))
	// Center the compact modal in the popup so the surrounding tmux popup
	// padding is even, and the box hugs its content instead of filling it.
	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, frame)
}

func renderUsageSection(windows []UsageWindow, empty string) string {
	if len(windows) == 0 {
		return uReset.Render(empty)
	}
	var b strings.Builder
	for i, w := range windows {
		if i > 0 {
			b.WriteString("\n\n")
		}
		b.WriteString(renderUsageLabel(w.Label) + "\n")
		bar := lipgloss.NewStyle().Foreground(barColor(w.Percent)).Render(renderBar(w.Percent, usageBarWidth))
		b.WriteString(bar + "  " + uPercent.Render(fmt.Sprintf("%d%%", w.Percent)) + uLabel.Render(" used"))
		if !w.ResetsAt.IsZero() {
			b.WriteString("\n" + uReset.Render("Resets "+w.ResetsAt.Local().Format("Mon Jan 2 3:04pm")))
		}
	}
	return b.String()
}

// usageWindows returns the windows for one source. Source-less snapshots are
// legacy Claude snapshots, retained for fixtures and old cache files.
func usageWindows(snap UsageSnapshot, source UsageSource) []UsageWindow {
	var windows []UsageWindow
	for _, w := range snap.Windows {
		if w.Source == source || (source == usageClaude && w.Source == "") {
			windows = append(windows, w)
		}
	}
	return windows
}

// renderUsageLabel colors the word "Fable" crimson wherever it appears; the
// rest of the label uses the muted label style.
func renderUsageLabel(label string) string {
	const fable = "Fable"
	i := strings.Index(label, fable)
	if i < 0 {
		return uLabel.Render(label)
	}
	return uLabel.Render(label[:i]) + uFable.Render(fable) + uLabel.Render(label[i+len(fable):])
}

// runUsage opens the usage popup (a read-only bubbletea program).
func runUsage() error {
	m := newUsageModel(newUsageProvider())
	p := tea.NewProgram(m, tea.WithAltScreen())
	_, err := p.Run()
	return err
}

// runUsageRaw prints the raw /api/oauth/usage response body to stdout — a
// one-time diagnostic for pinning the real JSON field names. The body carries
// no token, so this is safe to display. Not wired to a popup or bind-key.
func runUsageRaw() error {
	ctx, cancel := context.WithTimeout(context.Background(), usageFetchTimeout)
	defer cancel()
	body, err := fetchRawUsageBody(ctx)
	if len(body) > 0 {
		fmt.Println(string(body))
	}
	return err
}

// renderUsageIndicator renders both agents' compact switcher indicators.
func renderUsageIndicator(snap UsageSnapshot) string {
	var indicators []string
	for _, agent := range []struct {
		source UsageSource
		color  lipgloss.Color
	}{
		{usageClaude, claudeCodeOrange},
		{usageCodex, blue},
	} {
		if indicator := renderUsageIndicatorFor(agent.color, usageWindows(snap, agent.source)); indicator != "" {
			indicators = append(indicators, indicator)
		}
	}
	return strings.Join(indicators, uLabel.Render(" | "))
}

func renderUsageIndicatorFor(color lipgloss.Color, windows []UsageWindow) string {
	if len(windows) == 0 {
		return ""
	}
	segs := []string{lipgloss.NewStyle().Foreground(color).Bold(true).Render("✷")}
	for _, w := range windows {
		pct := lipgloss.NewStyle().Foreground(barColor(w.Percent)).Render(fmt.Sprintf("%d%%", w.Percent))
		segs = append(segs, usageTag(w.Label)+" "+pct)
	}
	return segs[0] + " " + strings.Join(segs[1:], uLabel.Render(" · "))
}

// tmuxUsageBadge renders current session/week usage with independent pressure
// colors. Errors stay silent; the provider normally serves its last-good cache
// when a refresh fails.
func tmuxUsageBadge(p UsageProvider) string {
	if p == nil {
		return ""
	}
	ctx, cancel := context.WithTimeout(context.Background(), usageFetchTimeout)
	defer cancel()
	snap, err := p.Fetch(ctx)
	if err != nil {
		return ""
	}
	var badges []string
	for _, agent := range []struct {
		source UsageSource
		color  lipgloss.Color
	}{
		{usageClaude, claudeCodeOrange},
		{usageCodex, blue},
	} {
		if badge := tmuxUsageBadgeFor(agent.color, usageWindows(snap, agent.source)); badge != "" {
			badges = append(badges, badge)
		}
	}
	if len(badges) == 0 {
		return ""
	}
	return "#[default]" + strings.Join(badges, "#[default] | ") + "#[default]"
}

func tmuxUsageBadgeFor(color lipgloss.Color, windows []UsageWindow) string {
	var sessionPercent, weekPercent int
	var haveSession, haveWeek bool
	for _, w := range windows {
		switch {
		case strings.EqualFold(strings.TrimSpace(w.Label), "Current session"):
			sessionPercent, haveSession = w.Percent, true
		case strings.EqualFold(strings.TrimSpace(w.Label), "Current week (all models)"),
			strings.EqualFold(strings.TrimSpace(w.Label), "Current week"):
			weekPercent, haveWeek = w.Percent, true
		}
	}
	if !haveSession && !haveWeek {
		return ""
	}
	status := fmt.Sprintf("#[fg=%s,bold]✷", color)
	if haveSession {
		status += fmt.Sprintf(" #[fg=%s,bold]s:%d%%", barColor(sessionPercent), sessionPercent)
	}
	if haveWeek {
		status += fmt.Sprintf(" #[fg=%s,bold]w:%d%%", barColor(weekPercent), weekPercent)
	}
	return status
}

// usageTag maps a window label to its colored tag ("session"/"week"/"fable"),
// with the fable tag in crimson to match the popup.
func usageTag(label string) string {
	switch {
	case strings.Contains(label, "Fable"):
		return uFable.Render("fable")
	case strings.Contains(label, "session"):
		return uLabel.Render("session")
	default:
		return uLabel.Render("week")
	}
}
