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

// fableCrimson highlights the word "Fable" wherever it appears. Distinct from
// the palette's pinkish `red` used for >=90% bar pressure.
var fableCrimson = lipgloss.Color("#DC143C")

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
	case errors.Is(err, context.DeadlineExceeded):
		return "network timeout"
	default:
		return "unexpected response"
	}
}

var (
	uTitle    = lipgloss.NewStyle().Foreground(text).Bold(true)
	uLabel    = lipgloss.NewStyle().Foreground(subtext0)
	uFable    = lipgloss.NewStyle().Foreground(fableCrimson).Bold(true)
	uReset    = lipgloss.NewStyle().Foreground(overlay0)
	uFooter   = lipgloss.NewStyle().Foreground(overlay0)
	uErrStyle = lipgloss.NewStyle().Foreground(red)
)

func (m usageModel) View() string {
	if m.width == 0 {
		return ""
	}
	var b strings.Builder
	b.WriteString(uTitle.Render("Claude usage"))
	b.WriteString("\n\n")
	switch m.state {
	case usageLoading:
		b.WriteString(uLabel.Render("Fetching usage…"))
	case usageErr:
		b.WriteString(uErrStyle.Render("Usage unavailable") + uLabel.Render(" — "+usageErrorReason(m.err)))
	case usageLoaded:
		for _, w := range m.snapshot.Windows {
			b.WriteString(renderUsageLabel(w.Label) + "\n")
			bar := lipgloss.NewStyle().Foreground(barColor(w.Percent)).Render(renderBar(w.Percent, usageBarWidth))
			line := fmt.Sprintf("%s  %d%% used", bar, w.Percent)
			if !w.ResetsAt.IsZero() {
				line += uReset.Render("   Resets " + w.ResetsAt.Local().Format("Jan 2 3:04pm"))
			}
			b.WriteString(line + "\n\n")
		}
	}
	b.WriteString("\n" + uFooter.Render("r refresh · esc close"))
	return b.String()
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
