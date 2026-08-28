package main

import (
	"os"
	"os/exec"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// switcherStartupBudget is what the switcher gets between launch and a fully
// rendered first frame.
const switcherStartupBudget = 250 * time.Millisecond

// TestSwitcherStartupBudget times everything the switcher does before its first
// paint and holds it to switcherStartupBudget.
//
// It measures the live tmux server under the machine's real load, so it is opt-in
// rather than part of the suite:
//
//	DOMUX_PROFILE=1 go test -run TestSwitcherStartupBudget -v -count=1
//
// The per-step breakdown is the point — it says which step regressed, not merely
// that something did. Every step should sit within a small multiple of the spawn
// floor; a step that grows with the number of open sessions is the bug, because
// that means a per-session process crept back into gatherSessions.
func TestSwitcherStartupBudget(t *testing.T) {
	if os.Getenv("DOMUX_PROFILE") == "" {
		t.Skip("set DOMUX_PROFILE=1 (measures the live tmux server)")
	}

	lap := func(label string, fn func()) time.Duration {
		start := time.Now()
		fn()
		d := time.Since(start)
		t.Logf("%-38s %8.1fms", label, float64(d.Microseconds())/1000)
		return d
	}

	// The floor every other step is read against: one fork+exec doing nothing.
	lap("spawn floor (tmux display-message)", func() {
		exec.Command("tmux", "display-message", "-p", "x").Output()
	})

	var sessions []liveSession
	lap("tmux list-sessions", func() {
		out, _ := exec.Command("tmux", "list-sessions", "-F",
			"#{session_name}\t#{session_attached}").Output()
		sessions = parseSessionListLines(string(out))
	})
	t.Logf("live sessions: %d", len(sessions))

	var snap tmuxServerSnapshot
	lap("readTmuxServerSnapshot", func() { snap = readTmuxServerSnapshot() })
	lap("readClaudeSessions", func() { readClaudeSessions() })

	var paths []string
	for _, s := range sessions {
		if p := snap.sessionPath(s.Name); p != "" {
			paths = append(paths, p)
		}
	}
	lap("gitFactsForPaths (concurrent)", func() { gitFactsForPaths(paths) })

	// The blocking path itself: rows, then the model, then the first frame.
	var rows []pickerRow
	total := lap("gatherSessions", func() { rows = gatherSessions() })
	t.Logf("rows: %d", len(rows))

	var m pickerModel
	total += lap("newPickerModel", func() { m = newPickerModel(rows) })

	sized, _ := m.Update(tea.WindowSizeMsg{Width: 180, Height: 50})
	m = sized.(pickerModel)
	total += lap("View (first paint)", func() { _ = m.View() })

	t.Logf("%-38s %8.1fms  (budget %v)", "STARTUP TOTAL",
		float64(total.Microseconds())/1000, switcherStartupBudget)
	if total > switcherStartupBudget {
		t.Errorf("switcher startup took %v, over the %v budget", total, switcherStartupBudget)
	}
}
