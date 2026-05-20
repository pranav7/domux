package main

import (
	"slices"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

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

func TestPickerWaitingStatesUseDotWithoutWaitingBadge(t *testing.T) {
	cases := []sessionInfo{
		{Name: "claude-session", Claude: "WAITING"},
		{Name: "codex-session", Codex: "WAITING"},
	}

	for _, tc := range cases {
		m := newPickerModel([]pickerRow{
			{Kind: rowSession, Session: &tc},
		})

		got := m.renderSession(pickerRow{Kind: rowSession, Session: &tc}, true)
		if !strings.Contains(got, "▎") {
			t.Fatalf("%s missing waiting marker: %q", tc.Name, got)
		}
		if strings.Contains(got, "CLAUDE WAITING") || strings.Contains(got, "CODEX WAITING") {
			t.Fatalf("%s rendered waiting badge: %q", tc.Name, got)
		}
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
