package main

import (
	"context"
	"errors"
	"fmt"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

const usageBarWidth = 20

// fableCrimson highlights the word "Fable" wherever it appears. This is the
// same muted, light-brick red the statusline uses for the Fable/Sonnet model
// name (~/dotfiles/claude/statusline-command.sh), distinct from the palette's
// pinkish `red` used for >=90% bar pressure.
var fableCrimson = lipgloss.Color("#C2797A")

// claudeCodeOrange is Claude Code's brand terracotta, used for the popup
// wordmark so the modal reads as an official Claude Code surface.
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
	uBrand    = lipgloss.NewStyle().Foreground(claudeCodeOrange).Bold(true)
	uTitle    = lipgloss.NewStyle().Foreground(subtext0)
	uLabel    = lipgloss.NewStyle().Foreground(subtext0)
	uPercent  = lipgloss.NewStyle().Foreground(text).Bold(true)
	uFable    = lipgloss.NewStyle().Foreground(fableCrimson).Bold(true)
	uReset    = lipgloss.NewStyle().Foreground(overlay0)
	uFooter   = lipgloss.NewStyle().Foreground(overlay0)
	uErrStyle = lipgloss.NewStyle().Foreground(red)
	// uFrame is a compact bordered modal with generous inner padding; it hugs
	// its content rather than filling the whole tmux popup.
	uFrame = lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(claudeCodeOrange).
		Padding(1, 3)
)

// blockGlyphs is a thin 3-row half-block font (same family as the picker's
// DOMUX logo) for the CLAUDE CODE wordmark. Half-block strokes (▀ ▄) keep the
// letters legible and separated at small size, unlike solid ██ bricks which
// merge into an unreadable mass in a terminal.
var blockGlyphs = map[rune][]string{
	'C': {"█▀▀", "█  ", "█▄▄"},
	'L': {"█  ", "█  ", "█▄▄"},
	'A': {"█▀█", "█▀█", "█ █"},
	'U': {"█ █", "█ █", "█▄█"},
	'D': {"█▀▄", "█ █", "█▄▀"},
	'E': {"█▀▀", "█▀ ", "█▄▄"},
	'O': {"█▀█", "█ █", "█▄█"},
}

const blockGlyphRows = 3

// renderBlockWord assembles a word into blockGlyphRows text rows, one glyph
// beside the next with a single-column gap.
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

// renderClaudeCodeLogo stacks CLAUDE over CODE in the brand terracotta
// half-block font, with a muted "usage" caption trailing the final row.
func renderClaudeCodeLogo() string {
	logo := lipgloss.NewStyle().Foreground(claudeCodeOrange).Bold(true)
	rows := append(renderBlockWord("CLAUDE"), renderBlockWord("CODE")...)
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
	// Block-art "CLAUDE CODE" wordmark in the brand terracotta, so the modal
	// reads as an official Claude Code surface.
	b.WriteString(renderClaudeCodeLogo())
	b.WriteString("\n")
	switch m.state {
	case usageLoading:
		b.WriteString(uLabel.Render("Fetching usage…"))
	case usageErr:
		b.WriteString(uErrStyle.Render("Usage unavailable") + uLabel.Render(" — "+usageErrorReason(m.err)))
	case usageLoaded:
		for i, w := range m.snapshot.Windows {
			b.WriteString(renderUsageLabel(w.Label) + "\n")
			bar := lipgloss.NewStyle().Foreground(barColor(w.Percent)).Render(renderBar(w.Percent, usageBarWidth))
			b.WriteString(bar + "  " + uPercent.Render(fmt.Sprintf("%d%%", w.Percent)) + uLabel.Render(" used") + "\n")
			// Reset time on its own line below the bar, indented under it.
			if !w.ResetsAt.IsZero() {
				b.WriteString(uReset.Render("Resets "+w.ResetsAt.Local().Format("Jan 2 3:04pm")) + "\n")
			}
			if i < len(m.snapshot.Windows)-1 {
				b.WriteString("\n")
			}
		}
	}
	b.WriteString("\n" + uFooter.Render("r refresh · esc close"))
	// Center the compact modal in the popup so the surrounding tmux popup
	// padding is even, and the box hugs its content instead of filling it.
	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, uFrame.Render(b.String()))
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

// renderUsageIndicator renders the compact top-right switcher indicator, e.g.
// "ses 15% · wk 24% · fab 4%", each percentage in its pressure color. Returns
// "" for an empty snapshot so the caller hides the indicator entirely — it
// never fabricates numbers.
func renderUsageIndicator(snap UsageSnapshot) string {
	if len(snap.Windows) == 0 {
		return ""
	}
	segs := make([]string, 0, len(snap.Windows))
	for _, w := range snap.Windows {
		pct := lipgloss.NewStyle().Foreground(barColor(w.Percent)).Render(fmt.Sprintf("%d%%", w.Percent))
		segs = append(segs, usageTag(w.Label)+" "+pct)
	}
	return strings.Join(segs, uLabel.Render(" · "))
}

// usageTag maps a window label to its short colored tag ("ses"/"wk"/"fab"),
// with the Fable tag in crimson to match the popup.
func usageTag(label string) string {
	switch {
	case strings.Contains(label, "Fable"):
		return uFable.Render("fab")
	case strings.Contains(label, "session"):
		return uLabel.Render("ses")
	default:
		return uLabel.Render("wk")
	}
}
