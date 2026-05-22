package main

import (
	"fmt"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

type utilityCommand struct {
	Name        string
	Description string
	IsOn        func() bool
	Toggle      func() error
}

var utilities = []utilityCommand{
	{
		Name:        "caffeinate",
		Description: "prevent idle sleep",
		IsOn:        caffeinateRunning,
		Toggle:      toggleCaffeinate,
	},
}

var (
	uLogo    = lipgloss.NewStyle().Foreground(mauve).Bold(true)
	uFeature = lipgloss.NewStyle().Foreground(overlay1).Italic(true)
	uName    = lipgloss.NewStyle().Foreground(text).Bold(true)
	uDesc    = lipgloss.NewStyle().Foreground(overlay1).Italic(true)
	uCursor  = lipgloss.NewStyle().Foreground(blue).Bold(true)
	uOn      = lipgloss.NewStyle().Foreground(green).Bold(true)
	uOff     = lipgloss.NewStyle().Foreground(overlay0)
	uOK      = lipgloss.NewStyle().Foreground(green)
	uErr     = lipgloss.NewStyle().Foreground(red)
	uSep     = lipgloss.NewStyle().Foreground(surface1).Render(" │ ")
	uKey     = lipgloss.NewStyle().Foreground(blue)
	uFoot    = lipgloss.NewStyle().Foreground(overlay0)
)

type utilitiesModel struct {
	cursor    int
	status    string
	statusErr bool
	startedAt time.Time
}

type utilitiesToggleMsg struct {
	Index int
	Err   error
}

func runUtilities() error {
	p := tea.NewProgram(newUtilitiesModel(), tea.WithAltScreen())
	_, err := p.Run()
	return err
}

func newUtilitiesModel() utilitiesModel {
	return utilitiesModel{startedAt: time.Now()}
}

func (m utilitiesModel) Init() tea.Cmd { return nil }

func (m utilitiesModel) ignoringStartupInput() bool {
	return !m.startedAt.IsZero() && time.Since(m.startedAt) < tuiStartupInputGrace
}

func (m utilitiesModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case utilitiesToggleMsg:
		if msg.Err != nil {
			m.status = fmt.Sprintf("%s: %v", utilities[msg.Index].Name, msg.Err)
			m.statusErr = true
		} else {
			state := "off"
			if utilities[msg.Index].IsOn() {
				state = "on"
			}
			m.status = fmt.Sprintf("%s %s", utilities[msg.Index].Name, state)
			m.statusErr = false
		}
		return m, nil
	case tea.KeyMsg:
		if m.ignoringStartupInput() {
			return m, nil
		}
		switch msg.String() {
		case "ctrl+c", "esc", "q":
			return m, tea.Quit
		case "up", "k":
			if m.cursor > 0 {
				m.cursor--
			}
			return m, nil
		case "down", "j":
			if m.cursor < len(utilities)-1 {
				m.cursor++
			}
			return m, nil
		case "enter", " ":
			idx := m.cursor
			if idx < 0 || idx >= len(utilities) {
				return m, nil
			}
			return m, func() tea.Msg {
				return utilitiesToggleMsg{Index: idx, Err: utilities[idx].Toggle()}
			}
		}
	}
	return m, nil
}

func (m utilitiesModel) View() string {
	var b strings.Builder
	b.WriteString("\n")
	b.WriteString("    " + uLogo.Render("█▀▄ █▀█ █▀▄▀█ █ █ ▀▄▀") + "\n")
	b.WriteString("    " + uLogo.Render("█▄▀ █▄█ █ ▀ █ █▄█ █ █") + "  " + uFeature.Render("commands") + "\n")
	b.WriteString("\n")

	for i, u := range utilities {
		glyph := "○"
		stateLabel := uOff.Render("off")
		glyphStyle := uOff
		if u.IsOn() {
			glyph = "●"
			stateLabel = uOn.Render("on")
			glyphStyle = uOn
		}
		cursor := "  "
		if i == m.cursor {
			cursor = " " + uCursor.Render("›")
		}
		line := fmt.Sprintf("   %s %s  %s  %s   %s",
			cursor,
			glyphStyle.Render(glyph),
			uName.Render(u.Name),
			uDesc.Render("— "+u.Description),
			stateLabel,
		)
		b.WriteString(line + "\n")
	}

	b.WriteString("\n")
	if m.status != "" {
		style := uOK
		if m.statusErr {
			style = uErr
		}
		b.WriteString("    " + style.Render(m.status) + "\n\n")
	}

	b.WriteString("    " +
		uKey.Render("↑↓") + uFoot.Render(" navigate") + uSep +
		uKey.Render("⏎") + uFoot.Render(" toggle") + uSep +
		uKey.Render("esc") + uFoot.Render(" close"))

	return b.String()
}
