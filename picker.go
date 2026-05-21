package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

type rowKind int

const (
	rowHeader rowKind = iota
	rowSpacer
	rowSession
	rowTask
)

type prInfo struct {
	Number int
	State  string
	Title  string
}

type sessionInfo struct {
	Name    string
	Branch  string
	PR      *prInfo
	Claude  string
	Codex   string
	Server  bool
	Windows int
	Path    string
	Root    string // git common root (group-level), stripped of /.baag/worktrees/...
	Label   string
	Tasks   []taskInfo
}

type taskInfo struct {
	Title       string
	InProgress  bool
	IsLast      bool
	SessionName string
	Path        string
}

type pickerRow struct {
	Kind    rowKind
	Group   string
	Session *sessionInfo
	Task    *taskInfo
}

type pickerModel struct {
	rows         []pickerRow
	visible      []int
	cursor       int
	filter       textinput.Model
	filtering    bool
	labelInput   textinput.Model
	labelEditing bool
	labelTarget  string
	showTasks    bool
	status       string
	statusErr    bool
	width        int
	height       int
	startedAt    time.Time
	spinnerFrame int
}

type pickerActionMsg struct {
	Action  string
	Session string
	Value   string
	Err     error
}

type pickerRefreshMsg struct {
	Rows []pickerRow
}

type pickerSpinnerMsg struct{}

const pickerStartupInputGrace = 150 * time.Millisecond
const pickerRefreshInterval = 2 * time.Second
const pickerSpinnerInterval = 150 * time.Millisecond
const claudeBrandHex = "#DE7356"

// claudeSpinnerFrames — star/asterisk shapes that pulse from sparse → dense → sparse,
// so each frame morphs into the next instead of just rotating.
var claudeSpinnerFrames = []string{"+", "✦", "✶", "✢", "✳", "✽", "✳", "✢", "✶", "✦"}

// Styles — Catppuccin Mocha
var (
	pTitle = lipgloss.NewStyle().
		Foreground(blue).
		Bold(true)

	pSubtitle = lipgloss.NewStyle().
			Foreground(overlay1)

	pGroupLabel = lipgloss.NewStyle().
			Foreground(overlay1).
			Bold(true)

	pGroupRule = lipgloss.NewStyle().
			Foreground(surface1)

	pCursor = lipgloss.NewStyle().
		Foreground(blue).
		Bold(true)

	pWaitingDot = lipgloss.NewStyle().
			Foreground(red).
			Bold(true)

	pName = lipgloss.NewStyle().
		Foreground(peach).
		Bold(true)

	pNameDim = lipgloss.NewStyle().
			Foreground(peach)

	pBadgeClauding = lipgloss.NewStyle().
			Foreground(lipgloss.Color(claudeBrandHex)).
			Bold(true)

	pSpinnerClaude = pBadgeClauding

	pBadgeCodexing = lipgloss.NewStyle().
			Foreground(blue).
			Bold(true)

	pSpinnerCodex = pBadgeCodexing

	pServer = lipgloss.NewStyle().
		Foreground(lipgloss.Color("#f9e2af"))

	pSep = lipgloss.NewStyle().
		Foreground(surface1)

	pBranch = lipgloss.NewStyle().
		Foreground(blue)

	pBranchDim = lipgloss.NewStyle().
			Foreground(overlay0)

	pPROpen   = lipgloss.NewStyle().Foreground(green).Bold(true)
	pPRMerged = lipgloss.NewStyle().Foreground(mauve).Bold(true)
	pPRClosed = lipgloss.NewStyle().Foreground(red).Bold(true)
	pPRDraft  = lipgloss.NewStyle().Foreground(overlay1).Bold(true)

	pTask = lipgloss.NewStyle().
		Foreground(overlay1).
		Italic(true)

	pTaskProgress = lipgloss.NewStyle().
			Foreground(yellow).
			Italic(true)

	pTaskMarker = lipgloss.NewStyle().
			Foreground(overlay0)

	pTaskProgressMarker = lipgloss.NewStyle().
				Foreground(yellow)

	pConnector = lipgloss.NewStyle().
			Foreground(overlay0)

	pFooter = lipgloss.NewStyle().
		Foreground(overlay0)

	pFooterKey = lipgloss.NewStyle().
			Foreground(blue)

	pFooterSep = lipgloss.NewStyle().
			Foreground(surface1)

	pStatus = lipgloss.NewStyle().
		Foreground(green)

	pStatusErr = lipgloss.NewStyle().
			Foreground(red)
)

func runPicker() error {
	rows := gatherSessions()
	m := newPickerModel(rows)
	p := tea.NewProgram(m, tea.WithAltScreen())
	_, err := p.Run()
	return err
}

func runPickerForSessionNames(names []string) error {
	keep := map[string]bool{}
	for _, name := range names {
		keep[name] = true
	}

	var filtered []pickerRow
	rows := gatherSessions()
	var pendingHeader *pickerRow
	for _, row := range rows {
		switch row.Kind {
		case rowHeader:
			copyRow := row
			pendingHeader = &copyRow
		case rowSession:
			if row.Session != nil && keep[row.Session.Name] {
				if pendingHeader != nil {
					filtered = append(filtered, *pendingHeader)
					pendingHeader = nil
				}
				filtered = append(filtered, row)
			}
		case rowTask:
			if row.Task != nil && keep[row.Task.SessionName] {
				filtered = append(filtered, row)
			}
		}
	}
	if len(filtered) == 0 {
		filtered = rows
	}
	m := newPickerModel(filtered)
	m.status = "matching sessions for this directory"
	p := tea.NewProgram(m, tea.WithAltScreen())
	_, err := p.Run()
	return err
}

func newPickerModel(rows []pickerRow) pickerModel {
	ti := textinput.New()
	ti.Placeholder = ""
	ti.CharLimit = 30

	li := textinput.New()
	li.Placeholder = ""
	li.CharLimit = 60

	m := pickerModel{
		rows:       rows,
		filter:     ti,
		labelInput: li,
		showTasks:  true,
		startedAt:  time.Now(),
	}
	m.rebuildVisible()
	for i, vi := range m.visible {
		if isSelectablePickerRow(m.rows[vi]) {
			m.cursor = i
			break
		}
	}
	return m
}

func (m *pickerModel) rebuildVisible() {
	filter := m.filter.Value()
	m.visible = m.visible[:0]

	if filter == "" {
		for i, r := range m.rows {
			if r.Kind == rowTask && !m.showTasks {
				continue
			}
			m.visible = append(m.visible, i)
		}
		return
	}

	filterLower := strings.ToLower(filter)
	matched := make(map[string]bool)
	for _, r := range m.rows {
		if r.Kind == rowSession && r.Session != nil {
			if strings.Contains(strings.ToLower(r.Session.Name), filterLower) {
				matched[r.Session.Name] = true
			}
		}
	}

	var headerIdx int
	spacerIdx := -1
	groupHasMatch := false
	for i, r := range m.rows {
		switch r.Kind {
		case rowSpacer:
			spacerIdx = i
		case rowHeader:
			headerIdx = i
			groupHasMatch = false
		case rowSession:
			if matched[r.Session.Name] {
				if !groupHasMatch {
					if len(m.visible) > 0 && spacerIdx >= 0 {
						m.visible = append(m.visible, spacerIdx)
					}
					m.visible = append(m.visible, headerIdx)
					groupHasMatch = true
				}
				m.visible = append(m.visible, i)
			}
		case rowTask:
			if m.showTasks && r.Task != nil && matched[r.Task.SessionName] {
				m.visible = append(m.visible, i)
			}
		}
	}
}

func isSelectablePickerRow(row pickerRow) bool {
	return row.Kind == rowSession && row.Session != nil
}

func (m pickerModel) Init() tea.Cmd {
	return tea.Batch(pickerRefreshCmd(), pickerSpinnerCmd())
}

func (m pickerModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil

	case pickerActionMsg:
		m.applyPickerAction(msg)
		return m, nil

	case pickerRefreshMsg:
		m.refreshRows(msg.Rows)
		return m, pickerRefreshCmd()

	case pickerSpinnerMsg:
		m.spinnerFrame = (m.spinnerFrame + 1) % len(claudeSpinnerFrames)
		return m, pickerSpinnerCmd()

	case tea.KeyMsg:
		key := msg.String()
		if m.ignoringStartupInput() {
			return m, nil
		}

		if m.labelEditing {
			switch key {
			case "ctrl+c":
				return m, tea.Quit
			case "esc":
				m.labelEditing = false
				m.labelInput.SetValue("")
				m.labelTarget = ""
				return m, nil
			case "enter":
				target := m.labelTarget
				value := strings.TrimSpace(m.labelInput.Value())
				m.labelEditing = false
				m.labelInput.SetValue("")
				m.labelTarget = ""
				if target == "" {
					return m, nil
				}
				m.status = fmt.Sprintf("labeling %s", target)
				m.statusErr = false
				return m, func() tea.Msg {
					return pickerActionMsg{
						Action:  "label",
						Session: target,
						Value:   value,
						Err:     setSessionLabel(target, value),
					}
				}
			case "backspace":
				v := m.labelInput.Value()
				if len(v) > 0 {
					m.labelInput.SetValue(v[:len(v)-1])
				}
				return m, nil
			default:
				if len(key) == 1 && key[0] >= 32 && key[0] < 127 {
					m.labelInput.SetValue(m.labelInput.Value() + key)
				}
				return m, nil
			}
		}

		if m.filtering {
			switch key {
			case "ctrl+c":
				return m, tea.Quit
			case "esc":
				m.filter.SetValue("")
				m.filtering = false
				m.rebuildVisible()
				m.clampCursor()
				return m, nil
			case "enter":
				if len(m.visible) == 0 {
					return m, nil
				}
				return m, m.selectRow(m.rows[m.visible[m.cursor]])
			case "up", "ctrl+p":
				m.moveCursor(-1)
				return m, nil
			case "down", "ctrl+n":
				m.moveCursor(1)
				return m, nil
			case "backspace":
				v := m.filter.Value()
				if len(v) > 0 {
					m.filter.SetValue(v[:len(v)-1])
					m.rebuildVisible()
					m.clampCursor()
				} else {
					m.filtering = false
				}
				return m, nil
			default:
				if len(key) == 1 && key[0] >= 32 && key[0] < 127 {
					m.filter.SetValue(m.filter.Value() + key)
					m.rebuildVisible()
					m.clampCursor()
				}
				return m, nil
			}
		}

		switch key {
		case "ctrl+c", "esc", "q":
			return m, tea.Quit
		case "enter":
			if len(m.visible) == 0 {
				return m, nil
			}
			return m, m.selectRow(m.rows[m.visible[m.cursor]])
		case "up", "k":
			m.moveCursor(-1)
			return m, nil
		case "down", "j":
			m.moveCursor(1)
			return m, nil
		case "tab":
			m.showTasks = !m.showTasks
			m.rebuildVisible()
			m.clampCursor()
			return m, nil
		case "/":
			m.filtering = true
			m.filter.SetValue("")
			return m, nil
		case "c":
			return m, m.clearSelectedSession()
		case "s":
			return m, m.setSelectedServer()
		case "n":
			m.startLabelEdit()
			return m, nil
		case "g":
			for i, vi := range m.visible {
				if isSelectablePickerRow(m.rows[vi]) {
					m.cursor = i
					break
				}
			}
			return m, nil
		case "G":
			for i := len(m.visible) - 1; i >= 0; i-- {
				if isSelectablePickerRow(m.rows[m.visible[i]]) {
					m.cursor = i
					break
				}
			}
			return m, nil
		case "+":
			return m, m.provisionInFocusedGroup()
		case "D":
			return m, m.deleteSelectedWorkspace()
		default:
			if len(key) == 1 && key[0] >= 32 && key[0] < 127 {
				m.filtering = true
				m.filter.SetValue(key)
				m.rebuildVisible()
				m.clampCursor()
				return m, nil
			}
		}
	}
	return m, nil
}

func pickerRefreshCmd() tea.Cmd {
	return tea.Tick(pickerRefreshInterval, func(time.Time) tea.Msg {
		return pickerRefreshMsg{Rows: gatherSessions()}
	})
}

func pickerSpinnerCmd() tea.Cmd {
	return tea.Tick(pickerSpinnerInterval, func(time.Time) tea.Msg {
		return pickerSpinnerMsg{}
	})
}

func (m *pickerModel) refreshRows(rows []pickerRow) {
	if len(rows) == 0 {
		return
	}
	selectedName := ""
	if len(m.visible) > 0 && m.cursor < len(m.visible) {
		row := m.rows[m.visible[m.cursor]]
		if row.Session != nil {
			selectedName = row.Session.Name
		}
	}
	m.rows = rows
	m.rebuildVisible()
	m.cursor = 0
	if selectedName != "" {
		for i, vi := range m.visible {
			row := m.rows[vi]
			if row.Session != nil && row.Session.Name == selectedName {
				m.cursor = i
				break
			}
		}
	}
	m.clampCursor()
}

func (m pickerModel) ignoringStartupInput() bool {
	return !m.startedAt.IsZero() && time.Since(m.startedAt) < pickerStartupInputGrace
}

func (m pickerModel) selectRow(row pickerRow) tea.Cmd {
	switch row.Kind {
	case rowSession:
		name := row.Session.Name
		return tea.Sequence(
			func() tea.Msg { switchSession(name); return nil },
			tea.Quit,
		)
	}
	return nil
}

func (m pickerModel) selectedSession() *sessionInfo {
	if len(m.visible) == 0 || m.cursor < 0 || m.cursor >= len(m.visible) {
		return nil
	}
	row := m.rows[m.visible[m.cursor]]
	if row.Kind != rowSession {
		return nil
	}
	return row.Session
}

func (m *pickerModel) clearSelectedSession() tea.Cmd {
	session := m.selectedSession()
	if session == nil {
		return nil
	}
	name := session.Name
	path := session.Path
	m.status = fmt.Sprintf("clearing %s", name)
	m.statusErr = false
	return func() tea.Msg {
		return pickerActionMsg{
			Action:  "clear",
			Session: name,
			Err:     clearWorkspaceForSession(name, path, false),
		}
	}
}

func (m *pickerModel) startLabelEdit() {
	session := m.selectedSession()
	if session == nil {
		return
	}
	m.labelEditing = true
	m.labelTarget = session.Name
	m.labelInput.SetValue(session.Label)
}

func (m *pickerModel) setSelectedServer() tea.Cmd {
	session := m.selectedSession()
	if session == nil {
		return nil
	}
	name := session.Name
	m.status = fmt.Sprintf("setting server to %s", name)
	m.statusErr = false
	return func() tea.Msg {
		return pickerActionMsg{
			Action:  "server",
			Session: name,
			Err:     setServerSessionByName(name),
		}
	}
}

func (m *pickerModel) provisionInFocusedGroup() tea.Cmd {
	session := m.selectedSession()
	if session == nil || session.Root == "" {
		m.status = "no git root for this row"
		m.statusErr = true
		return nil
	}
	root := session.Root
	group := m.rows[m.visible[m.cursor]].Group
	m.status = fmt.Sprintf("provisioning new workspace in %s", group)
	m.statusErr = false
	return func() tea.Msg {
		res, err := provisionWorkspace(root)
		return pickerActionMsg{
			Action:  "provision",
			Session: res.Session,
			Value:   res.Branch,
			Err:     err,
		}
	}
}

func (m *pickerModel) deleteSelectedWorkspace() tea.Cmd {
	// Real implementation lands in Task 11.
	return nil
}

func (m *pickerModel) applyPickerAction(msg pickerActionMsg) {
	if msg.Err != nil {
		switch msg.Action {
		case "clear":
			m.status = fmt.Sprintf("clear %s failed: %v", msg.Session, msg.Err)
		case "server":
			m.status = fmt.Sprintf("set server %s failed: %v", msg.Session, msg.Err)
		case "label":
			m.status = fmt.Sprintf("label %s failed: %v", msg.Session, msg.Err)
		case "provision":
			m.status = fmt.Sprintf("provision failed: %v", msg.Err)
		default:
			m.status = msg.Err.Error()
		}
		m.statusErr = true
		return
	}

	switch msg.Action {
	case "label":
		for _, row := range m.rows {
			if row.Kind == rowSession && row.Session != nil && row.Session.Name == msg.Session {
				row.Session.Label = msg.Value
			}
		}
		if msg.Value == "" {
			m.status = fmt.Sprintf("cleared label for %s", msg.Session)
		} else {
			m.status = fmt.Sprintf("labeled %s", msg.Session)
		}
	case "clear":
		for _, row := range m.rows {
			if row.Kind == rowSession && row.Session != nil && row.Session.Name == msg.Session {
				row.Session.Claude = ""
				row.Session.Label = ""
				row.Session.Server = false
			}
		}
		m.status = fmt.Sprintf("cleared %s", msg.Session)
	case "server":
		for _, row := range m.rows {
			if row.Kind == rowSession && row.Session != nil {
				row.Session.Server = row.Session.Name == msg.Session
			}
		}
		m.status = fmt.Sprintf("server set to %s", msg.Session)
	case "provision":
		m.status = fmt.Sprintf("provisioned %s", msg.Value)
		m.statusErr = false
	}
	m.statusErr = false
}

func (m *pickerModel) moveCursor(dir int) {
	if len(m.visible) == 0 {
		return
	}
	for i := m.cursor + dir; i >= 0 && i < len(m.visible); i += dir {
		if isSelectablePickerRow(m.rows[m.visible[i]]) {
			m.cursor = i
			return
		}
	}
}

func (m *pickerModel) clampCursor() {
	if len(m.visible) == 0 {
		m.cursor = 0
		return
	}
	if m.cursor >= len(m.visible) {
		m.cursor = len(m.visible) - 1
	}
	if isSelectablePickerRow(m.rows[m.visible[m.cursor]]) {
		return
	}
	for i := m.cursor + 1; i < len(m.visible); i++ {
		if isSelectablePickerRow(m.rows[m.visible[i]]) {
			m.cursor = i
			return
		}
	}
	for i := m.cursor - 1; i >= 0; i-- {
		if isSelectablePickerRow(m.rows[m.visible[i]]) {
			m.cursor = i
			return
		}
	}
	m.cursor = 0
}

func (m pickerModel) View() string {
	if m.width == 0 {
		return ""
	}

	var b strings.Builder

	// Heading — block art
	logoLines := []string{
		"█▀▄ █▀█ █▀▄▀█ █ █ ▀▄▀",
		"█▄▀ █▄█ █ ▀ █ █▄█ █ █",
	}
	logoStyle := lipgloss.NewStyle().Foreground(mauve).Bold(true)
	featureStyle := lipgloss.NewStyle().Foreground(overlay1).Italic(true)
	b.WriteString("\n")
	for i, line := range logoLines {
		rendered := "    " + logoStyle.Render(line)
		if i == 0 {
			b.WriteString(rendered + "\n")
		} else {
			tag := "  " + featureStyle.Render("switcher")
			b.WriteString(rendered + tag + "\n")
		}
	}
	if m.labelEditing {
		prefix := "label " + m.labelTarget + ": "
		b.WriteString("\n    " + lipgloss.NewStyle().Foreground(peach).Render(prefix))
		b.WriteString(lipgloss.NewStyle().Foreground(text).Render(m.labelInput.Value()))
		b.WriteString(lipgloss.NewStyle().Foreground(peach).Render("▌") + "\n")
	} else if m.filtering {
		b.WriteString("\n    " + lipgloss.NewStyle().Foreground(blue).Render("/") + " ")
		b.WriteString(lipgloss.NewStyle().Foreground(text).Render(m.filter.Value()))
		b.WriteString(lipgloss.NewStyle().Foreground(blue).Render("▌") + "\n")
	} else {
		b.WriteString("\n")
	}

	// logo(2) + blank(1) + prompt-or-blank(1) + footer(1) + blank(1) = 6, plus 1 spare
	availableHeight := m.height - 7
	if m.filtering || m.labelEditing {
		availableHeight--
	}
	if m.status != "" {
		availableHeight--
	}

	startIdx := 0
	if m.cursor >= startIdx+availableHeight {
		startIdx = m.cursor - availableHeight + 1
	}

	rendered := 0
	for i, vi := range m.visible {
		if i < startIdx {
			continue
		}
		if rendered >= availableHeight {
			break
		}
		b.WriteString(m.renderRow(m.rows[vi], i == m.cursor) + "\n")
		rendered++
	}
	for rendered < availableHeight {
		b.WriteString("\n")
		rendered++
	}

	if m.status != "" {
		style := pStatus
		if m.statusErr {
			style = pStatusErr
		}
		b.WriteString("    " + style.Render(m.status) + "\n")
	}

	// Footer
	sep := pFooterSep.Render(" │ ")
	todoLabel := "hide todos"
	if !m.showTasks {
		todoLabel = "show todos"
	}
	b.WriteString("    " +
		pFooterKey.Render("↑↓") + pFooter.Render(" navigate") + sep +
		pFooterKey.Render("⏎") + pFooter.Render(" switch") + sep +
		pFooterKey.Render("n") + pFooter.Render(" name") + sep +
		pFooterKey.Render("c") + pFooter.Render(" clear") + sep +
		pFooterKey.Render("s") + pFooter.Render(" server") + sep +
		pFooterKey.Render("tab") + pFooter.Render(" "+todoLabel) + sep +
		pFooterKey.Render("/") + pFooter.Render(" filter") + sep +
		pFooterKey.Render("esc") + pFooter.Render(" close"))

	return b.String()
}

func (m pickerModel) renderRow(row pickerRow, selected bool) string {
	switch row.Kind {
	case rowHeader:
		return m.renderHeader(row)
	case rowSpacer:
		return ""
	case rowSession:
		return m.renderSession(row, selected)
	case rowTask:
		return m.renderTask(row, selected)
	}
	return ""
}

func (m pickerModel) renderHeader(row pickerRow) string {
	label := pGroupLabel.Render(strings.ToUpper(row.Group))
	labelWidth := lipgloss.Width(label)
	ruleWidth := m.width - labelWidth - 8 - 2
	if ruleWidth < 1 {
		ruleWidth = 1
	}
	rule := "  " + pGroupRule.Render(strings.Repeat("─", ruleWidth))
	return "    " + label + rule
}

func (m pickerModel) renderSession(row pickerRow, selected bool) string {
	s := row.Session
	var line strings.Builder
	waiting := s.Claude == "WAITING" || s.Codex == "WAITING"
	active := s.Claude != "" || s.Codex != ""

	// Prefix: left accent bar for waiting, cursor arrow for selected
	if waiting {
		if selected {
			line.WriteString("  " + pWaitingDot.Render("▎") + pCursor.Render("›") + " ")
		} else {
			line.WriteString("  " + pWaitingDot.Render("▎") + "  ")
		}
	} else if selected {
		line.WriteString("   " + pCursor.Render("›") + " ")
	} else {
		line.WriteString("     ")
	}

	// Name — selected or active rows should be visibly readable.
	if selected || active {
		line.WriteString(lipgloss.NewStyle().Foreground(text).Bold(true).Render(s.Name))
	} else {
		line.WriteString(lipgloss.NewStyle().Foreground(subtext0).Render(s.Name))
	}

	// Label (e.g. "Client Portal") — peach, it's the meaningful project name
	if s.Label != "" {
		if active {
			line.WriteString(pSep.Render(" · ") + pName.Render(s.Label))
		} else {
			line.WriteString(pSep.Render(" · ") + pNameDim.Render(s.Label))
		}
	}

	// Badge (after name, inline)
	line.WriteString(renderAIBadges(s.Claude, s.Codex, m.spinnerFrame))

	// Server
	if s.Server {
		line.WriteString(" " + pServer.Render("⚡"))
	}

	// Separator + branch (always colored — it's navigation context)
	if s.Branch != "" {
		line.WriteString(pSep.Render(" · ") + pBranch.Render(s.Branch))
	}

	// PR — number colored by state, title dimmed
	if s.PR != nil {
		pr := fmt.Sprintf("PR#%d", s.PR.Number)
		var prStyle lipgloss.Style
		switch s.PR.State {
		case "OPEN":
			prStyle = pPROpen
		case "MERGED":
			prStyle = pPRMerged
		case "CLOSED":
			prStyle = pPRClosed
		case "DRAFT":
			prStyle = pPRDraft
		default:
			prStyle = pPROpen
		}
		line.WriteString(pSep.Render(" · ") + prStyle.Render(pr))
		if s.PR.Title != "" {
			title := s.PR.Title
			if len(title) > 40 {
				title = title[:37] + "..."
			}
			line.WriteString(pSep.Render(" · ") + pSubtitle.Render(title))
		}
	}

	result := line.String()
	return result
}

func renderAIBadges(claude, codex string, spinnerFrame int) string {
	var line strings.Builder
	frame := claudeSpinnerFrames[spinnerFrame%len(claudeSpinnerFrames)]
	switch claude {
	case "CLAUDING":
		line.WriteString(" " + pSpinnerClaude.Render(frame) + " " + pBadgeClauding.Render("Clauding"))
	}
	switch codex {
	case "CODEXING":
		line.WriteString(" " + pSpinnerCodex.Render(frame) + " " + pBadgeCodexing.Render("Codexing"))
	}
	return line.String()
}

func (m pickerModel) renderTask(row pickerRow, _ bool) string {
	t := row.Task
	connector := "├─"
	if t.IsLast {
		connector = "└─"
	}

	marker := pTaskMarker.Render("○")
	title := pTask.Render(t.Title)
	if t.InProgress {
		marker = pTaskProgressMarker.Render("●")
		title = pTaskProgress.Render(t.Title)
	}

	line := "        " + pConnector.Render(connector) + " " + marker + " " + title

	return line
}

// Actions

func switchSession(name string) {
	_ = attachTmuxSession(name)
}

// Data gathering

func gatherSessions() []pickerRow {
	out, err := exec.Command("tmux", "list-sessions", "-F", "#{session_name}").Output()
	if err != nil {
		return nil
	}

	sessions := strings.Split(strings.TrimSpace(string(out)), "\n")
	if len(sessions) == 0 {
		return nil
	}

	var entries []groupEntry
	homeDir, _ := os.UserHomeDir()

	for _, sess := range sessions {
		if sess == "" {
			continue
		}

		info := &sessionInfo{Name: sess}
		state := loadSessionStateWithLegacy(sess)

		pathOut, err := exec.Command("tmux", "display-message", "-t", sess, "-p", "#{pane_current_path}").Output()
		panePath := ""
		if err == nil {
			panePath = strings.TrimSpace(string(pathOut))
		}
		if state.Root != "" {
			info.Path = state.Root
		} else {
			info.Path = panePath
		}

		if info.Path != "" {
			branchOut, err := exec.Command("git", "-C", info.Path, "branch", "--show-current").Output()
			if err == nil {
				info.Branch = strings.TrimSpace(string(branchOut))
			}
		}

		// Group by git root
		group := ""
		if info.Path != "" {
			rootOut, err := exec.Command("git", "-C", info.Path, "rev-parse", "--show-toplevel").Output()
			if err == nil {
				gitRoot := strings.TrimSpace(string(rootOut))
				if idx := strings.Index(gitRoot, "/.baag/worktrees/"); idx >= 0 {
					info.Root = gitRoot[:idx]
				} else {
					info.Root = gitRoot
				}
				group = filepath.Base(info.Root)
			}
		}
		if group == "" {
			if info.Path != "" {
				group = filepath.Base(info.Path)
			} else {
				group = "other"
			}
		}

		// PR cache
		prFile := filepath.Join(homeDir, ".tmux-pr-"+sess)
		if prData, err := os.ReadFile(prFile); err == nil {
			parts := strings.SplitN(strings.TrimSpace(string(prData)), "::", 3)
			if len(parts) == 3 {
				num := 0
				fmt.Sscanf(parts[0], "%d", &num)
				info.PR = &prInfo{Number: num, State: parts[1], Title: parts[2]}
			}
		}

		aiStates := aggregateAIStatesFromSession(state)
		info.Claude = aiStates.Claude
		info.Codex = aiStates.Codex
		info.Server = state.Server

		winOut, err := exec.Command("tmux", "list-windows", "-t", sess).Output()
		if err == nil {
			info.Windows = len(strings.Split(strings.TrimSpace(string(winOut)), "\n"))
		}

		info.Label = state.Label

		// Tasks
		if info.Path != "" {
			taskPath := state.TodoPath
			var err error
			if taskPath == "" {
				taskPath, err = resolvePath(info.Path)
			}
			if err == nil {
				list, err := loadList(taskPath)
				if err == nil && len(list.Active) > 0 {
					maxTasks := 5
					if len(list.Active) < maxTasks {
						maxTasks = len(list.Active)
					}
					for i := 0; i < maxTasks; i++ {
						info.Tasks = append(info.Tasks, taskInfo{
							Title:       list.Active[i].Title,
							InProgress:  list.Active[i].InProgress,
							IsLast:      i == maxTasks-1,
							SessionName: sess,
							Path:        info.Path,
						})
					}
				}
			}
		}

		entries = append(entries, groupEntry{group: group, session: info})
	}

	return rowsFromEntries(entries)
}

func rowsFromEntries(entries []groupEntry) []pickerRow {
	sortEntries(entries)

	var rows []pickerRow
	currentGroup := ""
	for _, e := range entries {
		if e.group != currentGroup {
			if currentGroup != "" {
				rows = append(rows, pickerRow{Kind: rowSpacer, Group: e.group})
			}
			rows = append(rows, pickerRow{Kind: rowHeader, Group: e.group})
			currentGroup = e.group
		}
		rows = append(rows, pickerRow{Kind: rowSession, Group: e.group, Session: e.session})
		for i := range e.session.Tasks {
			rows = append(rows, pickerRow{Kind: rowTask, Group: e.group, Task: &e.session.Tasks[i]})
		}
	}

	return rows
}

func aggregateClaudeState(homeDir, session string) string {
	pattern := filepath.Join(homeDir, ".tmux-claude-"+session+"_*")
	matches, _ := filepath.Glob(pattern)

	hasWaiting := false
	hasClauding := false

	for _, f := range matches {
		data, err := os.ReadFile(f)
		if err != nil {
			continue
		}
		state := strings.TrimSpace(string(data))
		switch state {
		case "WAITING":
			hasWaiting = true
		case "CLAUDING":
			hasClauding = true
		}
	}

	// Only fall back to legacy file if no per-pane files exist AND file is fresh (<2min)
	if len(matches) == 0 {
		legacyFile := filepath.Join(homeDir, ".tmux-claude-"+session)
		if info, err := os.Stat(legacyFile); err == nil {
			if time.Since(info.ModTime()) < 2*time.Minute {
				if data, err := os.ReadFile(legacyFile); err == nil {
					state := strings.TrimSpace(string(data))
					switch state {
					case "WAITING":
						hasWaiting = true
					case "CLAUDING":
						hasClauding = true
					}
				}
			}
		}
	}

	if hasWaiting {
		return "WAITING"
	}
	if hasClauding {
		return "CLAUDING"
	}
	return ""
}

func sortEntries(entries []groupEntry) {
	for i := 1; i < len(entries); i++ {
		for j := i; j > 0; j-- {
			if entries[j].group < entries[j-1].group ||
				(entries[j].group == entries[j-1].group && entries[j].session.Name < entries[j-1].session.Name) {
				entries[j], entries[j-1] = entries[j-1], entries[j]
			} else {
				break
			}
		}
	}
}

type groupEntry struct {
	group   string
	session *sessionInfo
}
