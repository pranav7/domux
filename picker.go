package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
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
	Name        string
	Branch      string
	PR          *prInfo
	Claude      string
	Codex       string
	ClaudeLabel string
	CodexLabel  string
	Server      bool
	Windows     int
	Path        string
	Root        string // git common root (group-level), stripped of scratch worktree dirs
	Label       string
	Tasks       []taskInfo
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
	rows          []pickerRow
	visible       []int
	cursor        int
	filter        textinput.Model
	filtering     bool
	labelInput    textinput.Model
	labelEditing  bool
	labelTarget   string
	confirmDelete bool
	deleteAction  string
	deleteSession string
	deleteSlot    int
	deleteRoot    string
	deleteBranch  string
	deleteForce   bool
	showTasks     bool
	status        string
	statusErr     bool
	statusSetAt   time.Time
	width         int
	height        int
	startedAt     time.Time
	spinnerFrame  int
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

type pickerStatusExpireMsg struct{ at time.Time }

const tuiStartupInputGrace = 150 * time.Millisecond
const pickerRefreshInterval = 2 * time.Second
const pickerSpinnerInterval = 80 * time.Millisecond
const pickerStatusTTL = 5 * time.Second
const claudeBrandHex = "#DE7356"

// claudeSpinnerFrames — star/asterisk shapes that pulse from sparse → dense → sparse,
// so each frame morphs into the next instead of just rotating.
var claudeSpinnerFrames = []string{"·", "✦", "✶", "✳", "✢", "✻", "✽", "✻", "✢", "✳", "✶", "✦", "·"}

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
		Foreground(green).
		Bold(true)

	pNameDim = lipgloss.NewStyle().
			Foreground(green)

	pBadgeClauding = lipgloss.NewStyle().
			Foreground(lipgloss.Color(claudeBrandHex)).
			Bold(true)

	pSpinnerClaude = pBadgeClauding

	pBadgeCodexing = lipgloss.NewStyle().
			Foreground(blue).
			Bold(true)

	pSpinnerCodex = pBadgeCodexing

	pSpinnerCompacting = lipgloss.NewStyle().
				Foreground(compactPurple).
				Bold(true)

	pServer = lipgloss.NewStyle().
		Foreground(lipgloss.Color("#f9e2af"))

	pSep = lipgloss.NewStyle().
		Foreground(surface1)

	pBranch = lipgloss.NewStyle().
		Foreground(pink)

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

	pMainMark = lipgloss.NewStyle().
			Foreground(overlay0)

	pFooter = lipgloss.NewStyle().
		Foreground(overlay0)

	pFooterKey = lipgloss.NewStyle().
			Foreground(blue)

	pFooterSep = lipgloss.NewStyle().
			Foreground(surface1)

	pStatus = lipgloss.NewStyle().
		Foreground(base).
		Background(green).
		Bold(true).
		Padding(0, 1)

	pStatusErr = lipgloss.NewStyle().
			Foreground(base).
			Background(red).
			Bold(true).
			Padding(0, 1)
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
	m.statusSetAt = time.Now()
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

// isMainWorktreePath returns true when path looks like the main checkout of a
// project (i.e. not nested under a known scratch worktree dir).
func isMainWorktreePath(path string) bool {
	if path == "" {
		return false
	}
	return !isKnownWorkspacePath(path)
}

func (m pickerModel) Init() tea.Cmd {
	cmds := []tea.Cmd{pickerRefreshCmd(), pickerSpinnerCmd()}
	if m.status != "" && !m.statusSetAt.IsZero() {
		cmds = append(cmds, statusExpireCmd(m.statusSetAt))
	}
	return tea.Batch(cmds...)
}

func statusExpireCmd(at time.Time) tea.Cmd {
	return tea.Tick(pickerStatusTTL, func(time.Time) tea.Msg {
		return pickerStatusExpireMsg{at: at}
	})
}

func (m pickerModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	prevStatus := m.status
	nm, cmd := m.updateInner(msg)
	npm := nm.(pickerModel)
	if npm.status != "" && npm.status != prevStatus {
		now := time.Now()
		npm.statusSetAt = now
		expire := statusExpireCmd(now)
		if cmd == nil {
			cmd = expire
		} else {
			cmd = tea.Batch(cmd, expire)
		}
	}
	return npm, cmd
}

func (m pickerModel) updateInner(msg tea.Msg) (tea.Model, tea.Cmd) {
	if msg, ok := msg.(pickerStatusExpireMsg); ok {
		if m.statusSetAt.Equal(msg.at) {
			m.status = ""
			m.statusErr = false
		}
		return m, nil
	}
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
		// Wrap at LCM-ish large number so both the icon (mod 10) and the
		// shimmer wave (variable cycle) read smoothly.
		m.spinnerFrame = (m.spinnerFrame + 1) % 600
		return m, pickerSpinnerCmd()

	case tea.KeyMsg:
		key := msg.String()
		if m.ignoringStartupInput() {
			return m, nil
		}

		if m.confirmDelete {
			switch key {
			case "ctrl+c":
				return m, tea.Quit
			case "y", "Y":
				m.confirmDelete = false
				action := m.deleteAction
				if action == "" {
					action = "delete"
				}
				target := m.deleteBranch
				m.statusErr = false

				switch action {
				case "close":
					session := m.deleteSession
					m.status = fmt.Sprintf("closing %s", session)
					return m, func() tea.Msg {
						return pickerActionMsg{
							Action:  "close",
							Session: session,
							Err:     closeTmuxSession(session),
						}
					}
				default:
					session := m.deleteSession
					if session == "" {
						session = target
					}
					root := m.deleteRoot
					slot := m.deleteSlot
					force := m.deleteForce
					m.status = fmt.Sprintf("removing %s", target)
					return m, func() tea.Msg {
						return pickerActionMsg{
							Action:  "delete",
							Session: session,
							Value:   target,
							Err:     removeWorkspace(root, slot, force),
						}
					}
				}
			default:
				action := m.deleteAction
				if action == "" {
					action = "delete"
				}
				m.confirmDelete = false
				m.status = action + " cancelled"
				m.statusErr = false
				return m, nil
			}
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
				m.filtering = false
				m.clampCursor()
				return m, nil
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
		case "r":
			return m, m.resetSelectedBranch()
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
			return m, m.deleteOrCloseSelectedSession()
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
	return !m.startedAt.IsZero() && time.Since(m.startedAt) < tuiStartupInputGrace
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

func (m *pickerModel) resetSelectedBranch() tea.Cmd {
	session := m.selectedSession()
	if session == nil {
		return nil
	}
	name := session.Name
	path := session.Path
	if path == "" {
		m.status = fmt.Sprintf("no path for %s", name)
		m.statusErr = true
		return nil
	}
	m.status = fmt.Sprintf("resetting branch for %s", name)
	m.statusErr = false
	return func() tea.Msg {
		return pickerActionMsg{
			Action:  "reset",
			Session: name,
			Err:     resetGitWorkspace(path, false),
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

func (m *pickerModel) deleteOrCloseSelectedSession() tea.Cmd {
	session := m.selectedSession()
	if session == nil {
		return nil
	}
	dir := filepath.Base(session.Path)

	if session.Root == "" || !isWorkspaceDir(dir) {
		m.confirmDelete = true
		m.deleteAction = "close"
		m.deleteSession = session.Name
		m.deleteSlot = 0
		m.deleteRoot = ""
		m.deleteBranch = ""
		m.deleteForce = false
		m.status = fmt.Sprintf("close %s? (y/N)", session.Name)
		m.statusErr = false
		return nil
	}

	slot, err := strconv.Atoi(strings.TrimPrefix(dir, "workspace-"))
	if err != nil || slot < 1 {
		m.status = fmt.Sprintf("unrecognised workspace dir: %s", dir)
		m.statusErr = true
		return nil
	}
	m.confirmDelete = true
	m.deleteAction = "delete"
	m.deleteSession = session.Name
	m.deleteSlot = slot
	m.deleteRoot = session.Root
	m.deleteBranch = dir
	m.deleteForce = false
	m.status = fmt.Sprintf("delete %s? (y/N)", dir)
	m.statusErr = false
	return nil
}

func (m *pickerModel) applyPickerAction(msg pickerActionMsg) {
	if msg.Err == errDirtyWorkspace && msg.Action == "delete" {
		m.confirmDelete = true
		m.deleteForce = true
		m.status = fmt.Sprintf("%s has unpushed work — force delete? (y/N)", pickerActionTarget(msg))
		m.statusErr = true
		return
	}
	if msg.Err == errClearDirty && (msg.Action == "clear" || msg.Action == "reset") {
		m.status = fmt.Sprintf("%s has uncommitted changes — commit or stash first", msg.Session)
		m.statusErr = true
		return
	}
	if msg.Err != nil {
		switch msg.Action {
		case "clear":
			m.status = fmt.Sprintf("clear %s failed: %v", msg.Session, msg.Err)
		case "reset":
			m.status = fmt.Sprintf("reset branch for %s failed: %v", msg.Session, msg.Err)
		case "server":
			m.status = fmt.Sprintf("set server %s failed: %v", msg.Session, msg.Err)
		case "label":
			m.status = fmt.Sprintf("label %s failed: %v", msg.Session, msg.Err)
		case "provision":
			m.status = fmt.Sprintf("provision failed: %v", msg.Err)
		case "close":
			m.status = fmt.Sprintf("close %s failed: %v", msg.Session, msg.Err)
		case "delete":
			m.status = fmt.Sprintf("delete %s failed: %v", pickerActionTarget(msg), msg.Err)
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
	case "reset":
		m.status = fmt.Sprintf("reset branch for %s", msg.Session)
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
	case "delete":
		m.removeSessionRows(msg.Session)
		m.status = fmt.Sprintf("removed %s", pickerActionTarget(msg))
		m.statusErr = false
	case "close":
		m.removeSessionRows(msg.Session)
		m.status = fmt.Sprintf("closed %s", msg.Session)
		m.statusErr = false
	}
	m.statusErr = false
}

func pickerActionTarget(msg pickerActionMsg) string {
	if msg.Value != "" {
		return msg.Value
	}
	return msg.Session
}

func (m *pickerModel) removeSessionRows(session string) {
	var entries []groupEntry
	for _, row := range m.rows {
		if row.Kind != rowSession || row.Session == nil || row.Session.Name == session {
			continue
		}
		entries = append(entries, groupEntry{group: row.Group, session: row.Session})
	}
	m.rows = rowsFromEntries(entries)
	m.rebuildVisible()
	m.clampCursor()
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

func (m pickerModel) renderLabelOverlay() string {
	innerWidth := 36
	value := m.labelInput.Value()
	title := lipgloss.NewStyle().Foreground(peach).Bold(true).Render("name session")
	target := lipgloss.NewStyle().Foreground(overlay1).Render(m.labelTarget)
	inputLine := lipgloss.NewStyle().Foreground(text).Render(value) +
		lipgloss.NewStyle().Foreground(peach).Render("▌")
	hint := lipgloss.NewStyle().Foreground(overlay0).Render("enter to confirm · esc to cancel")
	box := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(peach).
		Padding(1, 2).
		Width(innerWidth).
		Render(title + "\n" + target + "\n\n" + inputLine + "\n\n" + hint)
	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, box)
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

	statusBox := ""
	if m.status != "" {
		style := pStatus
		if m.statusErr {
			style = pStatusErr
		}
		statusBox = style.Render(m.status)
	}

	b.WriteString("\n")
	for i, line := range logoLines {
		rendered := "    " + logoStyle.Render(line)
		var trailing string
		if i == 1 {
			trailing = "  " + featureStyle.Render("switcher")
		}
		if i == 0 && statusBox != "" {
			usedWidth := lipgloss.Width(rendered)
			statusWidth := lipgloss.Width(statusBox)
			pad := m.width - usedWidth - statusWidth - 4
			if pad < 1 {
				pad = 1
			}
			b.WriteString(rendered + strings.Repeat(" ", pad) + statusBox + "\n")
		} else {
			b.WriteString(rendered + trailing + "\n")
		}
	}
	showFilterLine := m.filtering || m.filter.Value() != ""
	if m.labelEditing {
		return m.renderLabelOverlay()
	} else if showFilterLine {
		b.WriteString("\n    " + lipgloss.NewStyle().Foreground(blue).Render("/") + " ")
		b.WriteString(lipgloss.NewStyle().Foreground(text).Render(m.filter.Value()))
		if m.filtering {
			b.WriteString(lipgloss.NewStyle().Foreground(blue).Render("▌"))
		}
		b.WriteString("\n")
	} else {
		b.WriteString("\n")
	}

	// logo(2) + blank(1) + prompt-or-blank(1) + footer(1) + blank(1) = 6, plus 1 spare
	availableHeight := m.height - 7
	if showFilterLine {
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

	// Footer
	sep := pFooterSep.Render(" │ ")
	todoLabel := "hide todos"
	if !m.showTasks {
		todoLabel = "show todos"
	}
	b.WriteString("    " +
		pFooterKey.Render("↑↓") + pFooter.Render(" navigate") + sep +
		pFooterKey.Render("⏎") + pFooter.Render(" switch") + sep +
		pFooterKey.Render("+") + pFooter.Render(" new") + sep +
		pFooterKey.Render("D") + pFooter.Render(" close/delete") + sep +
		pFooterKey.Render("n") + pFooter.Render(" name") + sep +
		pFooterKey.Render("c") + pFooter.Render(" clear") + sep +
		pFooterKey.Render("r") + pFooter.Render(" reset") + sep +
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

	mainGlyph := " "
	if isMainWorktreePath(s.Path) {
		mainGlyph = pMainMark.Render("◇")
	}
	// Prefix: left accent bar for waiting, cursor arrow for selected, then
	// the main-worktree marker. Five-column total to keep alignment with
	// non-session rows.
	switch {
	case waiting && selected:
		line.WriteString("  " + pWaitingDot.Render("▎") + pCursor.Render("›") + mainGlyph)
	case waiting:
		line.WriteString("  " + pWaitingDot.Render("▎") + " " + mainGlyph)
	case selected:
		line.WriteString("   " + pCursor.Render("›") + mainGlyph)
	default:
		line.WriteString("    " + mainGlyph)
	}
	line.WriteString(" ")

	nameStyle := lipgloss.NewStyle().Foreground(teal)
	if selected || active {
		nameStyle = nameStyle.Bold(true)
	}
	labelStyle := pNameDim
	if active || selected {
		labelStyle = pName
	}

	// Order: {name} on {branch} | {label} ⚡ {AI}
	line.WriteString(nameStyle.Render(s.Name))

	if s.Branch != "" {
		line.WriteString(pSep.Render(" on ") + pBranch.Render(s.Branch))
	}

	if s.Label != "" {
		line.WriteString(pSep.Render(" | ") + labelStyle.Render(s.Label))
	}

	// Server
	if s.Server {
		line.WriteString(" " + pServer.Render("⚡"))
	}

	// AI badge (spinner + working label)
	line.WriteString(renderAIBadges(s.Claude, s.Codex, s.ClaudeLabel, s.CodexLabel, m.spinnerFrame))

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

// Shimmer endpoints — dim/bright pair that the wave interpolates between.
// Kept off-brand-dim so the trailing chars fade out without going invisible.
const (
	claudeShimmerDim     = "#B85E47"
	claudeShimmerBright  = "#FFC9B0"
	codexShimmerDim      = "#6478A8"
	codexShimmerBright   = "#C8DAFF"
	compactShimmerDim    = "#6F6FCF"
	compactShimmerBright = "#D8D8FF"
)

func renderAIBadges(claude, codex, claudeLabel, codexLabel string, spinnerFrame int) string {
	var line strings.Builder
	// Icon advances every 2 ticks (~160ms).
	frame := claudeSpinnerFrames[(spinnerFrame/2)%len(claudeSpinnerFrames)]
	// COMPACTING short-circuits — render once with a fixed "Compacting…" label
	// regardless of which agent slot carries it. Suppress the per-agent working
	// badges since the same agent is mid-compaction, not working.
	if claude == "COMPACTING" || codex == "COMPACTING" {
		line.WriteString(" " + pSpinnerCompacting.Render(frame) + " " + shimmerText("Compacting…", spinnerFrame, compactShimmerDim, compactShimmerBright))
		return line.String()
	}
	usedLabels := map[string]bool{}
	switch claude {
	case "CLAUDING":
		if claudeLabel == "" {
			claudeLabel = stableAIWorkingLabelExcept("claude:"+claude+":"+codex, usedLabels)
		}
		usedLabels[claudeLabel] = true
		line.WriteString(" " + pSpinnerClaude.Render(frame) + " " + shimmerText(claudeLabel, spinnerFrame, claudeShimmerDim, claudeShimmerBright))
	}
	switch codex {
	case "CODEXING":
		if codexLabel == "" || usedLabels[codexLabel] {
			codexLabel = stableAIWorkingLabelExcept("codex:"+claude+":"+codex, usedLabels)
		}
		line.WriteString(" " + pSpinnerCodex.Render(frame) + " " + shimmerText(codexLabel, spinnerFrame, codexShimmerDim, codexShimmerBright))
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
				if root, ok := workspaceRootFromPath(gitRoot); ok {
					info.Root = root
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
		info.ClaudeLabel = aiStates.ClaudeLabel
		info.CodexLabel = aiStates.CodexLabel
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
