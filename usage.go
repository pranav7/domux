package main

import (
	"strings"

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
