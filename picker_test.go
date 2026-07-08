package main

import (
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/muesli/termenv"
)

var testANSIRE = regexp.MustCompile(`\x1b\[[0-9;]*m`)

func stripTestANSI(s string) string { return testANSIRE.ReplaceAllString(s, "") }

func TestPickerIgnoresInitialEscape(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "test"},
		{Kind: rowSession, Session: &sessionInfo{Name: "test"}},
	})

	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEsc})
	if cmd != nil {
		t.Fatalf("initial Esc should be ignored")
	}
}

func TestPickerEscapeQuitsAfterStartup(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "test"},
		{Kind: rowSession, Session: &sessionInfo{Name: "test"}},
	})
	time.Sleep(200 * time.Millisecond)

	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEsc})
	if cmd == nil {
		t.Fatalf("Esc after startup should quit")
	}
}

func TestPickerNStartsLabelEdit(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "audrey-app", Label: "PBC v1"}},
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	pm := next.(pickerModel)
	if !pm.labelEditing {
		t.Fatalf("n should enter label-edit mode")
	}
	if pm.labelTarget != "audrey-app" {
		t.Fatalf("labelTarget = %q, want audrey-app", pm.labelTarget)
	}
	if pm.labelInput.Value() != "PBC v1" {
		t.Fatalf("labelInput pre-fill = %q, want PBC v1", pm.labelInput.Value())
	}
}

func TestPickerHelpOverlayToggle(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "audrey-app"}},
	})
	m.width, m.height = 120, 40
	time.Sleep(200 * time.Millisecond)

	// '?' opens the overlay rather than starting a filter for "?"
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'?'}})
	pm := next.(pickerModel)
	if !pm.helpOpen {
		t.Fatalf("? should open help overlay")
	}
	if pm.filtering || pm.filter.Value() != "" {
		t.Fatalf("? must not start a filter, got filtering=%v value=%q", pm.filtering, pm.filter.Value())
	}
	if !strings.Contains(pm.View(), "keybindings") {
		t.Fatalf("help overlay should render the cheatsheet")
	}

	// any key closes it (esc here)
	next, _ = pm.Update(tea.KeyMsg{Type: tea.KeyEsc})
	pm = next.(pickerModel)
	if pm.helpOpen {
		t.Fatalf("esc should close help overlay")
	}
}

func TestPickerLabelEditEscCancels(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s", Label: "old"}},
	})
	time.Sleep(200 * time.Millisecond)
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	pm := next.(pickerModel)
	pm.labelInput.SetValue("new")

	next, _ = pm.Update(tea.KeyMsg{Type: tea.KeyEsc})
	pm = next.(pickerModel)
	if pm.labelEditing {
		t.Fatalf("Esc should exit label-edit mode")
	}
	if pm.labelInput.Value() != "" {
		t.Fatalf("labelInput should be cleared, got %q", pm.labelInput.Value())
	}
}

func TestPickerLabelEditLocksOutFilter(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	time.Sleep(200 * time.Millisecond)
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	pm := next.(pickerModel)

	next, _ = pm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'/'}})
	pm = next.(pickerModel)
	if pm.filtering {
		t.Fatalf("/ should not enter filter while label-editing")
	}
	if pm.labelInput.Value() != "/" {
		t.Fatalf("/ should be typed into label input, got %q", pm.labelInput.Value())
	}
}

func TestPickerLabelActionUpdatesRowImmediately(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s", Label: "old"}},
	})

	m.applyPickerAction(pickerActionMsg{Action: "label", Session: "s", Value: "new"})
	if got := m.rows[1].Session.Label; got != "new" {
		t.Fatalf("row Label = %q, want new", got)
	}
}

func TestPickerResetKeyDispatchesReset(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s", Path: "/tmp/s"}},
	})
	time.Sleep(200 * time.Millisecond)

	next, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'r'}})
	pm := next.(pickerModel)
	if cmd == nil {
		t.Fatalf("r should dispatch a reset cmd")
	}
	if !strings.Contains(pm.status, "resetting branch") {
		t.Fatalf("status = %q, want reset status", pm.status)
	}
}

func TestPickerResetActionKeepsSessionState(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{
			Name:   "s",
			Label:  "PBC",
			Server: true,
			Claude: "CLAUDING",
		}},
	})

	m.applyPickerAction(pickerActionMsg{Action: "reset", Session: "s"})
	row := m.rows[1].Session
	if row.Label != "PBC" || !row.Server || row.Claude != "CLAUDING" {
		t.Fatalf("reset action changed session state: %#v", row)
	}
}

func TestPickerClearActionClearsSessionStateImmediately(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{
			Name:        "s",
			Label:       "PBC",
			Server:      true,
			Claude:      "CLAUDING",
			Codex:       "CODEXING",
			ClaudeLabel: "Thinking",
			CodexLabel:  "Working",
			PR:          &prInfo{Number: 315, State: "MERGED", Title: "fix"},
		}},
	})

	m.applyPickerAction(pickerActionMsg{Action: "clear", Session: "s"})
	row := m.rows[1].Session
	if row.Label != "" || row.Server || row.Claude != "" || row.Codex != "" || row.ClaudeLabel != "" || row.CodexLabel != "" || row.PR != nil {
		t.Fatalf("clear action left stale session state: %#v", row)
	}
}

func TestPickerResetRefusesRowWithoutPath(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})

	cmd := m.resetSelectedBranch()
	if cmd != nil {
		t.Fatalf("reset without path should not dispatch")
	}
	if !m.statusErr || !strings.Contains(m.status, "no path") {
		t.Fatalf("status = %q, err = %v; want no path error", m.status, m.statusErr)
	}
}

func TestPickerRefreshRowsUpdatesSessionState(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "domux"},
		{Kind: rowSession, Group: "domux", Session: &sessionInfo{Name: "dotfiles"}},
	})

	m.refreshRows([]pickerRow{
		{Kind: rowHeader, Group: "domux"},
		{Kind: rowSession, Group: "domux", Session: &sessionInfo{Name: "dotfiles", Codex: "CODEXING"}},
	})

	row := m.rows[m.visible[m.cursor]]
	if row.Session == nil || row.Session.Codex != "CODEXING" {
		t.Fatalf("refreshed Codex state = %#v", row.Session)
	}
}

func TestPickerKeepsRelatedGroupsSeparate(t *testing.T) {
	rows := rowsFromEntries([]groupEntry{
		{group: "audrey-app", session: &sessionInfo{Name: "audrey-app"}},
		{group: "audrey", session: &sessionInfo{Name: "audrey"}},
	})

	var groups []string
	for _, row := range rows {
		if row.Kind == rowHeader {
			groups = append(groups, row.Group)
		}
	}

	want := []string{"audrey", "audrey-app"}
	if !slices.Equal(groups, want) {
		t.Fatalf("groups = %#v, want %#v", groups, want)
	}
	if rows[2].Kind != rowSpacer {
		t.Fatalf("row[2] kind = %v, want spacer", rows[2].Kind)
	}
}

func TestPickerFilterSkipsLeadingSpacer(t *testing.T) {
	m := newPickerModel(rowsFromEntries([]groupEntry{
		{group: "audrey-app", session: &sessionInfo{Name: "audrey-app"}},
		{group: "domux", session: &sessionInfo{Name: "domux"}},
		{group: "dotfiles", session: &sessionInfo{Name: "dotfiles"}},
	}))
	m.filter.SetValue("dotfiles")
	m.rebuildVisible()

	if len(m.visible) == 0 {
		t.Fatalf("visible rows empty")
	}
	if got := m.rows[m.visible[0]].Kind; got == rowSpacer {
		t.Fatalf("first visible row is spacer")
	}
}

func TestPickerFilterEnterAllowsShortcutsOnFilteredList(t *testing.T) {
	m := newPickerModel(rowsFromEntries([]groupEntry{
		{group: "g", session: &sessionInfo{Name: "alpha"}},
		{group: "g", session: &sessionInfo{Name: "domux", Path: "/tmp/domux"}},
	}))
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'/'}})
	pm := next.(pickerModel)
	next, _ = pm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'d'}})
	pm = next.(pickerModel)
	next, cmd := pm.Update(tea.KeyMsg{Type: tea.KeyEnter})
	pm = next.(pickerModel)

	if cmd != nil {
		t.Fatalf("enter should exit filter input, not switch")
	}
	if pm.filtering {
		t.Fatalf("enter should exit filter input")
	}
	if pm.filter.Value() != "d" {
		t.Fatalf("filter = %q, want d", pm.filter.Value())
	}
	session := pm.selectedSession()
	if session == nil {
		t.Fatalf("selected session is nil")
	}
	if got := session.Name; got != "domux" {
		t.Fatalf("selected session = %q, want domux", got)
	}

	next, cmd = pm.updateInner(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'c'}})
	pm = next.(pickerModel)
	if cmd != nil {
		t.Fatalf("c should only open clear confirmation")
	}
	if !pm.confirmDelete || pm.deleteAction != "clear" {
		t.Fatalf("clear should enter confirm mode: confirm=%v action=%q", pm.confirmDelete, pm.deleteAction)
	}
	if pm.filtering || pm.filter.Value() != "d" {
		t.Fatalf("clear shortcut changed filter state: filtering=%v filter=%q", pm.filtering, pm.filter.Value())
	}
	pm.width = 80
	pm.height = 20
	if !strings.Contains(pm.View(), "clear session") {
		t.Fatalf("clear confirmation modal missing:\n%s", pm.View())
	}

	next, cmd = pm.updateInner(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	pm = next.(pickerModel)
	if cmd == nil {
		t.Fatalf("y should run clear")
	}
	if !strings.Contains(pm.status, "clearing domux") {
		t.Fatalf("status = %q, want clearing domux", pm.status)
	}
}

func TestPickerEnterRecordsSelectionAndQuits(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "audrey-app"}},
	})
	time.Sleep(200 * time.Millisecond)

	next, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	pm := next.(pickerModel)

	// Enter must record the chosen session and return tea.Quit — the attach is
	// deferred to runPickerProgram so it never runs while bubbletea owns the
	// tty. (We can't compare tea.Cmd values directly, but it must be non-nil.)
	if pm.selected != "audrey-app" {
		t.Fatalf("selected = %q, want audrey-app", pm.selected)
	}
	if cmd == nil {
		t.Fatalf("enter should return a quit command")
	}
}

func TestPickerEnterOnNonSessionRowDoesNothing(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Session: &sessionInfo{Name: "only"}},
	})
	// Selecting a header (non-session) row must not record a selection.
	pm, cmd := m.selectRow(pickerRow{Kind: rowHeader, Group: "g"})
	if pm.(pickerModel).selected != "" {
		t.Fatalf("selected = %q, want empty", pm.(pickerModel).selected)
	}
	if cmd != nil {
		t.Fatalf("selecting a non-session row should be a no-op")
	}
}

func TestPickerRightOpensPreview(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	time.Sleep(200 * time.Millisecond)

	next, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRight})
	pm := next.(pickerModel)
	if !pm.previewOpen {
		t.Fatalf("right should open preview")
	}
	if pm.previewSession != "s" || pm.previewTarget != "s" {
		t.Fatalf("preview target = %q/%q, want s/s", pm.previewSession, pm.previewTarget)
	}
	if cmd == nil {
		t.Fatalf("right should dispatch preview capture")
	}
}

func TestPickerEscClosesPreview(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	m.previewOpen = true
	m.previewSession = "s"
	m.previewTarget = "s"
	time.Sleep(200 * time.Millisecond)

	next, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEsc})
	pm := next.(pickerModel)
	if pm.previewOpen {
		t.Fatalf("esc should close preview")
	}
	if cmd != nil {
		t.Fatalf("esc closing preview should not quit")
	}
}

func TestPickerPreviewViewFitsHeight(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	m.width = 120
	m.height = 20
	m.previewOpen = true
	m.previewSession = "s"
	m.previewTarget = "s:1.0"
	m.previewLines = []string{"one", "two", "three"}

	if got := viewLineCount(m.View()); got > m.height {
		t.Fatalf("view lines = %d, want <= %d", got, m.height)
	}
	if view := m.View(); !strings.Contains(view, "╭") || !strings.Contains(view, "╰") {
		t.Fatalf("preview should render rounded border")
	}
}

func TestPickerFullPreviewToggle(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	time.Sleep(200 * time.Millisecond)

	next, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'F'}})
	pm := next.(pickerModel)
	if !pm.previewOpen || !pm.previewBig {
		t.Fatalf("F should open big preview")
	}
	if cmd == nil {
		t.Fatalf("F should dispatch preview capture")
	}

	next, _ = pm.Update(tea.KeyMsg{Type: tea.KeyEsc})
	pm = next.(pickerModel)
	if !pm.previewOpen || pm.previewBig {
		t.Fatalf("esc should shrink big preview first")
	}
}

func TestPickerBigPreviewUsesFullWidth(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	m.width = 120
	m.previewOpen = true
	m.previewBig = true
	m.previewSession = "s"
	m.previewTarget = "s:1.0"
	m.previewLines = []string{"one"}

	lines := m.renderContentLines(5)
	if got := lipgloss.Width(lines[0]); got != 116 {
		t.Fatalf("big preview width = %d, want 116", got)
	}
}

func TestPreferredPreviewPaneUsesAIPane(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"codex:2_3":  "CODEXING",
		"claude:1_0": "WAITING",
	}}

	if got := preferredPreviewPane(state); got != "1_0" {
		t.Fatalf("preferred pane = %q, want 1_0", got)
	}
}

func TestCaptureTmuxPreview(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" > "$DOMUX_TMUX_CALL"
printf '\033[31mone\033[0m\033[H\n\ntwo\n'
`, callFile)

	lines, err := captureTmuxPreview("s:1.0")
	if err != nil {
		t.Fatalf("captureTmuxPreview: %v", err)
	}
	want := []string{"\x1b[31mone\x1b[0m", "", "two"}
	if !slices.Equal(lines, want) {
		t.Fatalf("lines = %#v, want %#v", lines, want)
	}
	call := mustRead(t, callFile)
	if !strings.Contains(call, "capture-pane -ep -J -S -200 -t s:1.0") {
		t.Fatalf("tmux call = %q", call)
	}
}

func TestShellQuote(t *testing.T) {
	got := shellQuote("s:1.0'pane")
	if got != "'s:1.0'\"'\"'pane'" {
		t.Fatalf("shellQuote = %q", got)
	}
}

func TestPickerViewFitsHeightWithGroupHeaders(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{Name: "audrey-app"}},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{Name: "workspace-1"}},
		{Kind: rowHeader, Group: "audreyai_azure_tf_internal"},
		{Kind: rowSession, Group: "audreyai_azure_tf_internal", Session: &sessionInfo{Name: "audreyai_azure_tf_internal"}},
		{Kind: rowHeader, Group: "domux"},
		{Kind: rowSession, Group: "domux", Session: &sessionInfo{Name: "domux"}},
		{Kind: rowHeader, Group: "dotfiles"},
		{Kind: rowSession, Group: "dotfiles", Session: &sessionInfo{Name: "dotfiles"}},
	})
	m.width = 120
	m.height = 20

	if got := viewLineCount(m.View()); got > m.height {
		t.Fatalf("view lines = %d, want <= %d", got, m.height)
	}
}

func TestPickerViewOmitsHeaderSessionSummary(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "one", Claude: "CLAUDING"}},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "two"}},
	})
	m.width = 120
	m.height = 20

	view := m.View()
	if strings.Contains(view, "2 sessions") || strings.Contains(view, "1 active") {
		t.Fatalf("view rendered header summary:\n%s", view)
	}
}

func TestPickerWaitingStatesUseDotWithoutWaitingBadge(t *testing.T) {
	cases := []sessionInfo{
		{Name: "claude-session", Claude: "WAITING"},
		{Name: "codex-session", Codex: "WAITING"},
		{Name: "opencode-session", OpenCode: "WAITING"},
	}

	for _, tc := range cases {
		m := newPickerModel([]pickerRow{
			{Kind: rowSession, Session: &tc},
		})

		got := m.renderSession(pickerRow{Kind: rowSession, Session: &tc}, true)
		if !strings.Contains(got, "▎") {
			t.Fatalf("%s missing waiting marker: %q", tc.Name, got)
		}
		if strings.Contains(got, "CLAUDE WAITING") || strings.Contains(got, "CODEX WAITING") || strings.Contains(got, "OPENCODE WAITING") {
			t.Fatalf("%s rendered waiting badge: %q", tc.Name, got)
		}
	}
}

func TestWorkingBadgeShowsSpinnerFrameAndRandomLabel(t *testing.T) {
	frame0 := renderAIBadges("CLAUDING", "", "", "Calculating", "", 0)
	frame1 := renderAIBadges("CLAUDING", "", "", "Calculating", "", 1)
	frame3 := renderAIBadges("CLAUDING", "", "", "Calculating", "", 3)

	if !strings.Contains(frame0, claudeSpinnerFrames[0]) {
		t.Fatalf("frame 0 missing %q: %q", claudeSpinnerFrames[0], frame0)
	}
	// Icon advances every 3 ticks; shimmer advances every tick.
	if !strings.Contains(frame3, claudeSpinnerFrames[1]) {
		t.Fatalf("frame 3 missing %q: %q", claudeSpinnerFrames[1], frame3)
	}
	plain0 := stripTestANSI(frame0)
	if !strings.Contains(plain0, "Calculating") {
		t.Fatalf("badge missing stable working label: %q", plain0)
	}
	if strings.Contains(plain0, "Clauding") || strings.Contains(plain0, "Codexing") {
		t.Fatalf("badge should not use agent name text: %q", plain0)
	}
	if frame0 == frame3 {
		t.Fatalf("icon should advance every three ticks: %q == %q", frame0, frame3)
	}
	_ = frame1
	// Shimmer phase advances every tick. Force a color profile so lipgloss
	// emits the per-rune color codes that distinguish adjacent frames.
	prev := lipgloss.ColorProfile()
	lipgloss.SetColorProfile(termenv.TrueColor)
	defer lipgloss.SetColorProfile(prev)
	// Pick frames where the bright peak sits inside the word (the leading tail
	// is uniformly at the floor color, so early frames don't differ).
	s0 := shimmerText("Calculating", 6, claudeShimmerDim, claudeShimmerBright)
	s1 := shimmerText("Calculating", 7, claudeShimmerDim, claudeShimmerBright)
	if s0 == s1 {
		t.Fatalf("shimmer should advance between frames: %q == %q", s0, s1)
	}
	if claudeBrandHex != "#DE7356" {
		t.Fatalf("expected Claude brand colour #DE7356, got %q", claudeBrandHex)
	}
}

func TestShimmerBouncesBack(t *testing.T) {
	prev := lipgloss.ColorProfile()
	lipgloss.SetColorProfile(termenv.TrueColor)
	defer lipgloss.SetColorProfile(prev)

	outbound := shimmerText("Calculating", 10, claudeShimmerDim, claudeShimmerBright)
	inbound := shimmerText("Calculating", 105, claudeShimmerDim, claudeShimmerBright)
	if outbound != inbound {
		t.Fatalf("mirrored shimmer frames should match:\noutbound: %q\ninbound:  %q", outbound, inbound)
	}
}

func TestWorkingBadgesUseAgentLabels(t *testing.T) {
	got := stripTestANSI(renderAIBadges("CLAUDING", "CODEXING", "", "Pondering", "Computing", 0))
	if !strings.Contains(got, "Pondering") {
		t.Fatalf("missing Claude label: %q", got)
	}
	if !strings.Contains(got, "Computing") {
		t.Fatalf("missing Codex label: %q", got)
	}
}

func TestOpenCodeWorkingBadgeShowsPinkCoding(t *testing.T) {
	got := stripTestANSI(renderAIBadges("", "", "CODING", "", "", 0))
	if !strings.Contains(got, "Coding") {
		t.Fatalf("OpenCode badge should render fixed Coding label: %q", got)
	}
	if openCodePinkHex != "#C678B8" {
		t.Fatalf("OpenCode badge colour = %q, want #C678B8", openCodePinkHex)
	}
	prev := lipgloss.ColorProfile()
	lipgloss.SetColorProfile(termenv.TrueColor)
	defer lipgloss.SetColorProfile(prev)
	if renderAIBadges("", "", "CODING", "", "", 6) == renderAIBadges("", "", "CODING", "", "", 7) {
		t.Fatalf("OpenCode Coding shimmer should advance between frames")
	}
}

func containsAIWorkingLabel(s string) bool {
	for _, label := range aiWorkingLabels {
		if strings.Contains(s, label) {
			return true
		}
	}
	return false
}

func TestPickerSpinnerTickAdvancesFrame(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Session: &sessionInfo{Name: "s", Claude: "CLAUDING"}},
	})
	start := m.spinnerFrame
	next, _ := m.Update(pickerSpinnerMsg{})
	if next.(pickerModel).spinnerFrame == start {
		t.Fatalf("spinner frame did not advance")
	}
}

func TestTmuxAttachArgsUsesAttachOutsideTmux(t *testing.T) {
	got := tmuxAttachArgs(false, "dotfiles")
	want := []string{"attach-session", "-t", "dotfiles"}

	if !slices.Equal(got, want) {
		t.Fatalf("args = %#v", got)
	}
}

func viewLineCount(s string) int {
	if s == "" {
		return 0
	}
	return strings.Count(s, "\n") + 1
}

func TestPickerDeleteEntersConfirmMode(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "workspace-1",
			Path: "/r/.domux/worktrees/workspace-1",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)
	if !pm.confirmDelete {
		t.Fatalf("D should enter confirmDelete mode")
	}
	if pm.deleteSlot != 1 {
		t.Fatalf("deleteSlot = %d, want 1", pm.deleteSlot)
	}
}

func TestPickerDeleteClosesMainRow(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "audrey-app",
			Path: "/r",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)
	if !pm.confirmDelete {
		t.Fatalf("D on main row should enter confirmDelete")
	}
	if pm.deleteAction != "close" {
		t.Fatalf("deleteAction = %q, want close", pm.deleteAction)
	}
	if pm.deleteSession != "audrey-app" {
		t.Fatalf("deleteSession = %q, want audrey-app", pm.deleteSession)
	}
	if pm.statusErr || !strings.Contains(pm.status, "close audrey-app") {
		t.Fatalf("status = %q, err = %v", pm.status, pm.statusErr)
	}
}

func TestPickerDeleteCancelOnAnyOtherKey(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{
			Name: "workspace-1",
			Path: "/r/.domux/worktrees/workspace-1",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)

	next, _ = pm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	pm = next.(pickerModel)
	if pm.confirmDelete {
		t.Fatalf("non-y key should cancel confirmDelete")
	}
}

func TestPickerDeleteYDispatchesRemove(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{
			Name: "workspace-1",
			Path: "/r/.domux/worktrees/workspace-1",
			Root: "/r",
		}},
	})
	time.Sleep(200 * time.Millisecond)
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)

	next, cmd := pm.updateInner(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	pm = next.(pickerModel)
	if pm.confirmDelete {
		t.Fatalf("y should exit confirmDelete mode")
	}
	if cmd == nil {
		t.Fatalf("y should dispatch a remove cmd")
	}
	if !strings.Contains(pm.status, "removing") {
		t.Fatalf("status = %q, want it to mention removing", pm.status)
	}
}

func TestPickerCloseYDispatchesClose(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session|kill-session|refresh-client) exit 0 ;;
*) exit 1 ;;
esac
`, callFile)

	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{
			Name: "audrey-app",
			Path: "/r/audrey-app",
			Root: "/r/audrey-app",
		}},
	})
	time.Sleep(200 * time.Millisecond)
	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'D'}})
	pm := next.(pickerModel)

	next, cmd := pm.updateInner(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	pm = next.(pickerModel)
	if pm.confirmDelete {
		t.Fatalf("y should exit confirmDelete mode")
	}
	if cmd == nil {
		t.Fatalf("y should dispatch a close cmd")
	}

	msg := cmd().(pickerActionMsg)
	if msg.Action != "close" || msg.Session != "audrey-app" || msg.Err != nil {
		t.Fatalf("msg = %#v", msg)
	}
}

func TestPickerCloseActionRemovesEmptyGroup(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s", Tasks: []taskInfo{{Title: "todo", SessionName: "s"}}}},
		{Kind: rowTask, Group: "g", Task: &taskInfo{Title: "todo", SessionName: "s"}},
	})

	m.applyPickerAction(pickerActionMsg{Action: "close", Session: "s"})

	if len(m.rows) != 0 {
		t.Fatalf("rows = %#v, want empty", m.rows)
	}
}

func TestPickerClearActionRemovesTaskRows(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s", Tasks: []taskInfo{{Title: "todo", SessionName: "s"}}}},
		{Kind: rowTask, Group: "g", Task: &taskInfo{Title: "todo", SessionName: "s"}},
	})

	m.applyPickerAction(pickerActionMsg{Action: "clear", Session: "s"})

	for _, row := range m.rows {
		if row.Kind == rowTask {
			t.Fatalf("task row remained after clear: %#v", row)
		}
		if row.Kind == rowSession && row.Session != nil && len(row.Session.Tasks) != 0 {
			t.Fatalf("session tasks remained: %#v", row.Session.Tasks)
		}
	}
}

func TestPickerPlusDispatchesProvision(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "audrey-app",
			Root: "/tmp/audrey-app",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'+'}})
	if cmd == nil {
		t.Fatalf("+ should dispatch a cmd")
	}
}

func TestPickerPlusSetsProvisioningStatus(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "audrey-app"},
		{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
			Name: "audrey-app",
			Root: "/tmp/audrey-app",
		}},
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'+'}})
	pm := next.(pickerModel)
	if !strings.Contains(pm.status, "provisioning") {
		t.Fatalf("status = %q, want it to mention provisioning", pm.status)
	}
	if pm.statusErr {
		t.Fatalf("status should not be flagged as error")
	}
}

func TestRenderSessionMarksMainWorktree(t *testing.T) {
	mainRow := pickerRow{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
		Name: "audrey-app",
		Path: "/r/audrey-app",
		Root: "/r/audrey-app",
	}}
	wsRow := pickerRow{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
		Name: "workspace-1",
		Path: "/r/audrey-app/.domux/worktrees/workspace-1",
		Root: "/r/audrey-app",
	}}
	m := newPickerModel([]pickerRow{mainRow, wsRow})

	mainOut := m.renderSession(mainRow, false)
	wsOut := m.renderSession(wsRow, false)

	if !strings.Contains(mainOut, "◇") {
		t.Fatalf("main row missing ◇ glyph:\n%s", mainOut)
	}
	if strings.Contains(wsOut, "◇") {
		t.Fatalf("workspace row should not have ◇ glyph:\n%s", wsOut)
	}
}

func TestRenderSessionKeepsLabelInlineAndMovesPRToIndentedLine(t *testing.T) {
	row := pickerRow{Kind: rowSession, Group: "audrey-app", Session: &sessionInfo{
		Name:   "workspace-1",
		Branch: "feature/eng-147-v3",
		Label:  "PBC Validations",
		PR: &prInfo{
			Number: 311,
			State:  "OPEN",
			Title:  "feat(pbc): free-text validations",
		},
		Path: "/r/audrey-app/.domux/worktrees/workspace-1",
		Root: "/r/audrey-app",
	}}
	m := newPickerModel([]pickerRow{row})

	lines := strings.Split(stripTestANSI(m.renderSession(row, false)), "\n")
	if len(lines) != 2 {
		t.Fatalf("lines = %#v, want two lines", lines)
	}
	if !strings.Contains(lines[0], "PBC Validations") {
		t.Fatalf("label missing from first line: %q", lines[0])
	}
	if strings.Contains(lines[0], "PR#311") {
		t.Fatalf("PR leaked onto first line: %q", lines[0])
	}
	if !strings.HasPrefix(lines[1], "        ") {
		t.Fatalf("detail line not indented: %q", lines[1])
	}
	if !strings.Contains(lines[1], "PR#311 · feat(pbc): free-text validations") {
		t.Fatalf("detail line missing metadata: %q", lines[1])
	}
}

func TestRenderListLinesCountsSessionDetails(t *testing.T) {
	row := pickerRow{Kind: rowSession, Group: "g", Session: &sessionInfo{
		Name:  "workspace-1",
		Label: "PBC Validations",
		PR: &prInfo{
			Number: 311,
			State:  "OPEN",
			Title:  "feat(pbc): free-text validations",
		},
	}}
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		row,
	})

	lines := m.renderListLines(100, 3)
	plain := stripTestANSI(strings.Join(lines, "\n"))
	if !strings.Contains(plain, "PR#311") {
		t.Fatalf("detail line missing from rendered list:\n%s", plain)
	}
	if got := len(lines); got != 3 {
		t.Fatalf("lines = %d, want 3", got)
	}
}

func TestIsMainWorktreePathRecognizesCurrentAndLegacyDirs(t *testing.T) {
	cases := []struct {
		path string
		want bool
	}{
		{"/r/audrey-app", true},
		{"/r/audrey-app/.domux/worktrees/workspace-1", false},
		{"/r/audrey-app/.baag/worktrees/workspace-1", false},
	}
	for _, tc := range cases {
		if got := isMainWorktreePath(tc.path); got != tc.want {
			t.Fatalf("isMainWorktreePath(%q) = %v, want %v", tc.path, got, tc.want)
		}
	}
}

func TestPickerPlusIgnoresRowWithoutRoot(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "x"},
		{Kind: rowSession, Group: "x", Session: &sessionInfo{Name: "x"}}, // no Root
	})
	time.Sleep(200 * time.Millisecond)

	next, _ := m.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'+'}})
	pm := next.(pickerModel)
	if !pm.statusErr {
		t.Fatalf("expected error status, got %q", pm.status)
	}
	if pm.status == "" || !strings.Contains(pm.status, "press w to add a window") {
		t.Fatalf("expected guidance to mention 'press w to add a window', got %q", pm.status)
	}
}

func TestWrapWords(t *testing.T) {
	// Greedy wrap, no word split when each word fits.
	got := wrapWords("the quick brown fox jumps", 11)
	want := []string{"the quick", "brown fox", "jumps"}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Fatalf("wrapWords = %q, want %q", got, want)
	}
	for _, line := range got {
		if lipgloss.Width(line) > 11 {
			t.Fatalf("line %q exceeds width 11", line)
		}
	}
	// A single word wider than width is hard-split so nothing overflows.
	split := wrapWords(strings.Repeat("x", 25), 10)
	if len(split) != 3 || split[0] != strings.Repeat("x", 10) || split[2] != strings.Repeat("x", 5) {
		t.Fatalf("hard-split = %q, want 10/10/5", split)
	}
}

func TestRecapWrapsAcrossLinesUntruncated(t *testing.T) {
	recap := "PR #391 hooks client-portal and Files-tab uploads into document " +
		"categorization so TODO selectors find them; it's rebased and the conflict is gone"
	m := newPickerModel([]pickerRow{
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "ws", Recap: recap}},
	})
	m.width = 80
	m.showDetails = true

	width := 80
	lines := m.renderRowLines(m.rows[0], true, width)
	plain := stripTestANSI(strings.Join(lines, "\n"))

	if strings.Contains(plain, "…") {
		t.Fatalf("recap was truncated with ellipsis:\n%s", plain)
	}
	// Every word of the full recap must survive somewhere in the wrapped output.
	for _, word := range strings.Fields(recap) {
		if !strings.Contains(plain, word) {
			t.Fatalf("recap word %q missing from wrapped output:\n%s", word, plain)
		}
	}
	// And no rendered line may exceed the list width.
	for _, line := range lines {
		if lipgloss.Width(line) > width {
			t.Fatalf("rendered line exceeds width %d: %q", width, line)
		}
	}
	// The recap should actually wrap (more than the single first line + recap row).
	if len(lines) < 3 {
		t.Fatalf("expected recap to wrap onto multiple lines, got %d lines:\n%s", len(lines), plain)
	}
}

func TestResumeJobAdvancesSequentially(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	m.resume = &resumeJob{queue: []resumeTarget{
		{Name: "a", Prune: true},
		{Name: "b", Prune: true},
	}}

	n1, cmd1 := m.Update(resumeStepMsg{target: resumeTarget{Name: "a", Status: resumePruned}})
	m = n1.(pickerModel)
	if cmd1 == nil {
		t.Fatalf("expected a follow-up step cmd")
	}
	if m.resume.pos != 1 || m.resume.nPruned != 1 || m.resume.done {
		t.Fatalf("after step 1: pos=%d pruned=%d done=%v", m.resume.pos, m.resume.nPruned, m.resume.done)
	}

	n2, _ := m.Update(resumeStepMsg{target: resumeTarget{Name: "b", Status: resumePruned}})
	m = n2.(pickerModel)
	if !m.resume.done {
		t.Fatalf("expected done after final step")
	}
	if m.resume.nPruned != 2 {
		t.Fatalf("nPruned = %d, want 2", m.resume.nPruned)
	}
	if !strings.Contains(m.status, "pruned 2") {
		t.Fatalf("status = %q, want summary", m.status)
	}
}

func TestResumeBannerRendersProgress(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	m.resume = &resumeJob{queue: make([]resumeTarget, 2), pos: 1}

	banner := m.resumeBanner()
	if !strings.Contains(banner, "restoring") || !strings.Contains(banner, "/2") {
		t.Fatalf("banner = %q, want restoring N/2", banner)
	}

	m.resume.done = true
	if got := m.resumeBanner(); got != "" {
		t.Fatalf("done banner = %q, want empty", got)
	}
}

func TestParseWindowLines(t *testing.T) {
	out := "1\tprod uk\t0\t/Users/x/projects/audrey\n" +
		"2\tmerge queue\t1\t/Users/x/projects/audrey\n"
	got := parseWindowLines(out)
	if len(got) != 2 {
		t.Fatalf("got %d windows, want 2", len(got))
	}
	if got[0].Index != 1 || got[0].Name != "prod uk" || got[0].Active {
		t.Errorf("window 0 = %+v, want {Index:1 Name:%q Active:false}", got[0], "prod uk")
	}
	if got[1].Index != 2 || got[1].Name != "merge queue" || !got[1].Active {
		t.Errorf("window 1 = %+v, want {Index:2 Name:%q Active:true}", got[1], "merge queue")
	}
	if got[1].Path != "/Users/x/projects/audrey" {
		t.Errorf("window 1 Path = %q", got[1].Path)
	}
}

func TestParseWindowLinesEmpty(t *testing.T) {
	if got := parseWindowLines(""); len(got) != 0 {
		t.Errorf("empty output = %+v, want no windows", got)
	}
}

func TestParseWindowLinesMalformed(t *testing.T) {
	// A line with fewer than 4 tab-separated fields is skipped.
	out := "1\tshort\n2\tname\t1\t/path\n"
	got := parseWindowLines(out)
	if len(got) != 1 || got[0].Index != 2 {
		t.Errorf("short-line skip: got %+v, want only window 2", got)
	}
	// A line whose index is not a parseable integer is skipped.
	out = "x\tname\t0\t/path\n3\tgood\t1\t/path2\n"
	got = parseWindowLines(out)
	if len(got) != 1 || got[0].Index != 3 {
		t.Errorf("non-int index skip: got %+v, want only window 3", got)
	}
}

func TestRowsFromEntriesWindowRows(t *testing.T) {
	// >1 window → one rowWindow per window, under the session, above tasks.
	multi := &sessionInfo{
		Name: "domux", Path: "/p",
		Windows: []windowInfo{{Index: 1, Name: "a"}, {Index: 2, Name: "b"}},
		Tasks:   []taskInfo{{Title: "t1"}},
	}
	rows := rowsFromEntries([]groupEntry{{group: "domux", session: multi}})

	var kinds []rowKind
	for _, r := range rows {
		kinds = append(kinds, r.Kind)
	}
	// header, session, window, window, task
	want := []rowKind{rowHeader, rowSession, rowWindow, rowWindow, rowTask}
	if len(kinds) != len(want) {
		t.Fatalf("kinds = %v, want %v", kinds, want)
	}
	for i := range want {
		if kinds[i] != want[i] {
			t.Fatalf("kinds = %v, want %v", kinds, want)
		}
	}
}

func TestRowsFromEntriesSingleWindowNoWindowRows(t *testing.T) {
	single := &sessionInfo{
		Name: "solo", Path: "/p",
		Windows: []windowInfo{{Index: 1, Name: "a"}},
	}
	rows := rowsFromEntries([]groupEntry{{group: "solo", session: single}})
	for _, r := range rows {
		if r.Kind == rowWindow {
			t.Fatalf("single-window session must not emit rowWindow, got rows %+v", rows)
		}
	}
}

func TestRenderWindowActiveAndBadge(t *testing.T) {
	m := pickerModel{width: 120}
	active := pickerRow{Kind: rowWindow, Window: &windowInfo{
		Index: 2, Name: "merge queue", Active: true, Claude: "WAITING",
	}}
	out := m.renderWindow(active, false)
	if !strings.Contains(out, "2") || !strings.Contains(out, "merge queue") {
		t.Errorf("renderWindow missing index/name: %q", out)
	}
}

func TestRenderWindowRecapGatedByDetails(t *testing.T) {
	win := &windowInfo{Index: 1, Name: "a", Recap: "reshaped the pipeline"}

	off := pickerModel{width: 120, showDetails: false}
	if strings.Contains(off.renderWindow(pickerRow{Kind: rowWindow, Window: win}, false), "reshaped the pipeline") {
		t.Errorf("recap must be hidden when showDetails is off")
	}

	on := pickerModel{width: 120, showDetails: true}
	if !strings.Contains(on.renderWindow(pickerRow{Kind: rowWindow, Window: win}, false), "reshaped the pipeline") {
		t.Errorf("recap must show when showDetails is on")
	}
}

func TestSelectRowWindowTarget(t *testing.T) {
	sess := &sessionInfo{Name: "domux", Path: "/p",
		Windows: []windowInfo{{Index: 1, Name: "a"}, {Index: 2, Name: "b"}}}
	winRow := pickerRow{Kind: rowWindow, Session: sess, Window: &sess.Windows[1]}

	m := pickerModel{}
	updated, _ := m.selectRow(winRow)
	pm := updated.(pickerModel)
	if pm.selected != "domux:2" {
		t.Errorf("selected = %q, want domux:2", pm.selected)
	}
}

func TestSelectRowSessionTargetUnchanged(t *testing.T) {
	sess := &sessionInfo{Name: "domux"}
	m := pickerModel{}
	updated, _ := m.selectRow(pickerRow{Kind: rowSession, Session: sess})
	pm := updated.(pickerModel)
	if pm.selected != "domux" {
		t.Errorf("selected = %q, want domux", pm.selected)
	}
}

func TestWindowRowIsSelectable(t *testing.T) {
	sess := &sessionInfo{Name: "test"}
	if !isSelectablePickerRow(pickerRow{Kind: rowWindow, Session: sess, Window: &windowInfo{Index: 1}}) {
		t.Errorf("rowWindow should be selectable")
	}
}

func TestNewWindowArgs(t *testing.T) {
	got := newWindowArgs("domux", "merge queue", "/Users/x/projects/audrey")
	want := []string{"new-window", "-t", "domux", "-n", "merge queue", "-c", "/Users/x/projects/audrey"}
	if len(got) != len(want) {
		t.Fatalf("args = %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("args = %v, want %v", got, want)
		}
	}
}

func TestNewWindowArgsNoCwd(t *testing.T) {
	got := newWindowArgs("domux", "scratch", "")
	want := []string{"new-window", "-t", "domux", "-n", "scratch"}
	if len(got) != len(want) {
		t.Fatalf("args = %v, want %v", got, want)
	}
}

func TestWindowNamingFlow(t *testing.T) {
	sess := &sessionInfo{Name: "domux", Path: "/p", Windows: []windowInfo{{Index: 1}, {Index: 2}}}
	m := pickerModel{
		rows:    []pickerRow{{Kind: rowSession, Session: sess}},
		visible: []int{0},
		cursor:  0,
	}
	m.startWindowNaming()
	if !m.windowNaming || m.windowTarget != "domux" || m.windowCwd != "/p" {
		t.Fatalf("startWindowNaming state = naming:%v target:%q cwd:%q", m.windowNaming, m.windowTarget, m.windowCwd)
	}
}

func TestRefreshRowsRestoresWindowCursor(t *testing.T) {
	// Build initial rows with session + 2 windows
	sess := &sessionInfo{
		Name:    "domux",
		Path:    "/p",
		Windows: []windowInfo{{Index: 1, Name: "w1"}, {Index: 2, Name: "w2"}},
	}
	initialRows := rowsFromEntries([]groupEntry{{group: "domux", session: sess}})

	// Find window-index-2 row
	win2Idx := -1
	for i, r := range initialRows {
		if r.Kind == rowWindow && r.Window != nil && r.Window.Index == 2 {
			win2Idx = i
			break
		}
	}
	if win2Idx == -1 {
		t.Fatal("could not find window-index-2 row in initial rows")
	}

	// Build model with cursor on window 2
	m := pickerModel{rows: initialRows}
	m.rebuildVisible()
	m.cursor = 0
	for i, vi := range m.visible {
		if m.rows[vi].Kind == rowWindow && m.rows[vi].Window != nil && m.rows[vi].Window.Index == 2 {
			m.cursor = i
			break
		}
	}

	// Build fresh rows (same session, same windows)
	newSess := &sessionInfo{
		Name:    "domux",
		Path:    "/p",
		Windows: []windowInfo{{Index: 1, Name: "w1"}, {Index: 2, Name: "w2"}},
	}
	newRows := rowsFromEntries([]groupEntry{{group: "domux", session: newSess}})

	// Refresh
	m.refreshRows(newRows)

	// Cursor should still be on window-index-2, not on the session row
	if m.cursor >= len(m.visible) {
		t.Fatalf("cursor %d out of bounds (visible len %d)", m.cursor, len(m.visible))
	}
	row := m.rows[m.visible[m.cursor]]
	if row.Kind != rowWindow {
		t.Fatalf("after refresh, cursor on Kind %v, want rowWindow", row.Kind)
	}
	if row.Window == nil || row.Window.Index != 2 {
		t.Fatalf("after refresh, cursor on window %v, want Index=2", row.Window)
	}
}

func TestRefreshRowsRestoresSessionCursor(t *testing.T) {
	// Build rows with session + 2 windows
	sess := &sessionInfo{
		Name:    "domux",
		Path:    "/p",
		Windows: []windowInfo{{Index: 1, Name: "w1"}, {Index: 2, Name: "w2"}},
	}
	initialRows := rowsFromEntries([]groupEntry{{group: "domux", session: sess}})

	m := pickerModel{rows: initialRows}
	m.rebuildVisible()
	m.cursor = 0
	// Position cursor on the SESSION row
	for i, vi := range m.visible {
		if m.rows[vi].Kind == rowSession {
			m.cursor = i
			break
		}
	}

	// Build fresh rows
	newSess := &sessionInfo{
		Name:    "domux",
		Path:    "/p",
		Windows: []windowInfo{{Index: 1, Name: "w1"}, {Index: 2, Name: "w2"}},
	}
	newRows := rowsFromEntries([]groupEntry{{group: "domux", session: newSess}})

	// Refresh
	m.refreshRows(newRows)

	// Cursor should still be on the session row
	if m.cursor >= len(m.visible) {
		t.Fatalf("cursor %d out of bounds (visible len %d)", m.cursor, len(m.visible))
	}
	row := m.rows[m.visible[m.cursor]]
	if row.Kind != rowSession {
		t.Fatalf("after refresh, cursor on Kind %v, want rowSession", row.Kind)
	}
	if row.Session == nil || row.Session.Name != "domux" {
		t.Fatalf("after refresh, cursor on session %v, want domux", row.Session)
	}
}

func TestRenderSessionRecapSuppressedForMultiWindow(t *testing.T) {
	m := pickerModel{showDetails: true, width: 80}
	// Session with 2 windows
	sess := &sessionInfo{
		Name:    "domux",
		Recap:   "some recap text",
		Windows: []windowInfo{{Index: 1}, {Index: 2}},
	}
	row := pickerRow{Kind: rowSession, Session: sess}
	out := m.renderSession(row, false)
	if strings.Contains(out, "some recap text") {
		t.Fatal("renderSession included recap for multi-window session")
	}
}

func TestRenderSessionRecapShownForSingleWindow(t *testing.T) {
	m := pickerModel{showDetails: true, width: 80}
	// Session with 1 window (or 0)
	sess := &sessionInfo{
		Name:    "domux",
		Recap:   "some recap text",
		Windows: []windowInfo{{Index: 1}},
	}
	row := pickerRow{Kind: rowSession, Session: sess}
	out := m.renderSession(row, false)
	if !strings.Contains(out, "some recap text") {
		t.Fatal("renderSession omitted recap for single-window session")
	}

	// Also test with 0 windows
	sess.Windows = nil
	out = m.renderSession(row, false)
	if !strings.Contains(out, "some recap text") {
		t.Fatal("renderSession omitted recap for 0-window session")
	}
}

// The AI status badge belongs to the window it comes from. For a multi-window
// session it renders on the window row, not the session row, so it must not
// appear on both. The CLAUDING spinner glyph (frame 2 → claudeSpinnerFrames[1])
// only ever comes from renderAIBadges, so it is a reliable marker for "a badge
// was drawn here".
func TestRenderSessionBadgeSuppressedForMultiWindow(t *testing.T) {
	m := pickerModel{width: 80, spinnerFrame: 2}
	glyph := claudeSpinnerFrames[1]
	sess := &sessionInfo{
		Name:    "domux",
		Claude:  "CLAUDING",
		Windows: []windowInfo{{Index: 1}, {Index: 2, Claude: "CLAUDING"}},
	}
	out := m.renderSession(pickerRow{Kind: rowSession, Session: sess}, false)
	if strings.Contains(out, glyph) {
		t.Fatalf("renderSession drew an AI badge for a multi-window session: %q", out)
	}
}

func TestRenderSessionBadgeShownForSingleWindow(t *testing.T) {
	m := pickerModel{width: 80, spinnerFrame: 2}
	glyph := claudeSpinnerFrames[1]
	sess := &sessionInfo{
		Name:    "domux",
		Claude:  "CLAUDING",
		Windows: []windowInfo{{Index: 1, Claude: "CLAUDING"}},
	}
	out := m.renderSession(pickerRow{Kind: rowSession, Session: sess}, false)
	if !strings.Contains(out, glyph) {
		t.Fatalf("renderSession omitted the AI badge for a single-window session: %q", out)
	}

	// And with no windows at all (the common case), the badge still shows.
	sess.Windows = nil
	out = m.renderSession(pickerRow{Kind: rowSession, Session: sess}, false)
	if !strings.Contains(out, glyph) {
		t.Fatalf("renderSession omitted the AI badge for a 0-window session: %q", out)
	}
}
