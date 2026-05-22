package main

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

func TestUtilitiesViewShowsCaffeinate(t *testing.T) {
	m := utilitiesModel{startedAt: time.Now()}
	view := m.View()
	if !strings.Contains(view, "caffeinate") {
		t.Fatalf("view should contain caffeinate, got: %s", view)
	}
	if !(strings.Contains(view, "on") || strings.Contains(view, "off")) {
		t.Fatalf("view should show on/off state, got: %s", view)
	}
	if !strings.Contains(view, "toggle") {
		t.Fatalf("footer should mention toggle, got: %s", view)
	}
}

func TestUtilitiesIgnoresStartupInput(t *testing.T) {
	m := utilitiesModel{startedAt: time.Now()}
	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEsc})
	if cmd != nil {
		t.Fatalf("esc within startup grace should be ignored, got cmd")
	}
}

func TestUtilitiesEscAfterGraceQuits(t *testing.T) {
	m := utilitiesModel{startedAt: time.Now().Add(-2 * tuiStartupInputGrace)}
	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEsc})
	if cmd == nil {
		t.Fatalf("esc after grace should produce a quit cmd")
	}
}

func TestUtilitiesCursorClampedToList(t *testing.T) {
	m := utilitiesModel{startedAt: time.Now().Add(-2 * tuiStartupInputGrace)}
	for i := 0; i < len(utilities)+2; i++ {
		next, _ := m.Update(tea.KeyMsg{Type: tea.KeyDown})
		m = next.(utilitiesModel)
	}
	if m.cursor != len(utilities)-1 {
		t.Fatalf("cursor should clamp at last index %d, got %d", len(utilities)-1, m.cursor)
	}

	for i := 0; i < len(utilities)+5; i++ {
		next, _ := m.Update(tea.KeyMsg{Type: tea.KeyUp})
		m = next.(utilitiesModel)
	}
	if m.cursor != 0 {
		t.Fatalf("cursor should clamp at 0, got %d", m.cursor)
	}
}

func TestUtilitiesEnterTriggersToggleCmd(t *testing.T) {
	m := utilitiesModel{startedAt: time.Now().Add(-2 * tuiStartupInputGrace)}
	_, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	if cmd == nil {
		t.Fatalf("enter should return a toggle cmd")
	}
}

func TestUtilitiesToggleMsgUpdatesStatus(t *testing.T) {
	m := utilitiesModel{startedAt: time.Now()}
	next, _ := m.Update(utilitiesToggleMsg{Index: 0, Err: nil})
	nm := next.(utilitiesModel)
	if nm.status == "" {
		t.Fatalf("status should be set after toggle msg")
	}
	if nm.statusErr {
		t.Fatalf("statusErr should be false for nil err")
	}
}
