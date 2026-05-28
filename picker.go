package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

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
	Done        bool
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
	rows           []pickerRow
	visible        []int
	cursor         int
	filter         textinput.Model
	filtering      bool
	labelInput     textinput.Model
	labelEditing   bool
	labelTarget    string
	confirmDelete  bool
	deleteAction   string
	deleteSession  string
	deleteSlot     int
	deleteRoot     string
	deletePath     string
	deleteBranch   string
	deleteForce    bool
	showTasks      bool
	status         string
	statusErr      bool
	statusSetAt    time.Time
	width          int
	height         int
	startedAt      time.Time
	spinnerFrame   int
	previewOpen    bool
	previewSession string
	previewTarget  string
	previewLines   []string
	previewErr     error
	previewBig     bool
	helpOpen       bool
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

type pickerPRRefreshMsg struct {
	Rows []pickerRow
}

type pickerSpinnerMsg struct{}

type pickerPRRefreshTickMsg struct{}

type pickerStatusExpireMsg struct{ at time.Time }

type pickerPreviewMsg struct {
	Session string
	Target  string
	Lines   []string
	Err     error
}

type pickerPreviewTickMsg struct {
	Session string
	Target  string
}

type pickerPopupClosedMsg struct {
	Err error
}

const tuiStartupInputGrace = 150 * time.Millisecond
const pickerRefreshInterval = 2 * time.Second
const pickerPRRefreshInterval = 60 * time.Second
const pickerSpinnerInterval = 80 * time.Millisecond
const pickerPreviewInterval = 500 * time.Millisecond
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
			Foreground(surface0)

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

	pPROpen   = lipgloss.NewStyle().Foreground(green)
	pPRMerged = lipgloss.NewStyle().Foreground(mauve)
	pPRClosed = lipgloss.NewStyle().Foreground(red)
	pPRDraft  = lipgloss.NewStyle().Foreground(overlay1)

	pTask = lipgloss.NewStyle().
		Foreground(overlay1).
		Italic(true)

	pTaskProgress = lipgloss.NewStyle().
			Foreground(yellow).
			Italic(true)

	pTaskDone = lipgloss.NewStyle().
			Foreground(overlay0).
			Italic(true)

	pTaskMarker = lipgloss.NewStyle().
			Foreground(overlay0)

	pTaskProgressMarker = lipgloss.NewStyle().
				Foreground(yellow)

	pTaskDoneMarker = lipgloss.NewStyle().
			Foreground(green)

	pConnector = lipgloss.NewStyle().
			Foreground(surface1)

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

	pPreviewTitle = lipgloss.NewStyle().
			Foreground(blue).
			Bold(true)

	pPreviewMeta = lipgloss.NewStyle().
			Foreground(overlay1)

	pPreviewRule = lipgloss.NewStyle().
			Foreground(surface1)

	pPreviewText = lipgloss.NewStyle().
			Foreground(subtext0)

	pPreviewErr = lipgloss.NewStyle().
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
	cmds := []tea.Cmd{pickerRefreshCmd(), pickerSpinnerCmd(), pickerPRRefreshCmd()}
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

	case pickerPRRefreshMsg:
		m.refreshRows(msg.Rows)
		return m, pickerPRRefreshTickCmd()

	case pickerPRRefreshTickMsg:
		return m, pickerPRRefreshCmd()

	case pickerSpinnerMsg:
		// Wrap at LCM-ish large number so both the icon (mod 10) and the
		// shimmer wave (variable cycle) read smoothly.
		m.spinnerFrame = (m.spinnerFrame + 1) % 600
		return m, pickerSpinnerCmd()

	case pickerPreviewMsg:
		if !m.previewOpen || msg.Session != m.previewSession || msg.Target != m.previewTarget {
			return m, nil
		}
		m.previewLines = msg.Lines
		m.previewErr = msg.Err
		return m, pickerPreviewTickCmd(msg.Session, msg.Target)

	case pickerPreviewTickMsg:
		if !m.previewOpen || msg.Session != m.previewSession || msg.Target != m.previewTarget {
			return m, nil
		}
		return m, pickerPreviewRefreshCmd(msg.Session, msg.Target)

	case pickerPopupClosedMsg:
		if msg.Err != nil {
			m.status = fmt.Sprintf("preview popup failed: %v", msg.Err)
			m.statusErr = true
		}
		return m, windowSizeCmd(m.width, m.height)

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
				case "clear":
					session := m.deleteSession
					path := m.deletePath
					m.status = fmt.Sprintf("clearing %s", session)
					return m, func() tea.Msg {
						return pickerActionMsg{
							Action:  "clear",
							Session: session,
							Err:     clearWorkspaceForSession(session, path, false),
						}
					}
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

		if m.helpOpen {
			switch key {
			case "ctrl+c":
				return m, tea.Quit
			default:
				m.helpOpen = false
				return m, nil
			}
		}

		switch key {
		case "left":
			if m.previewOpen {
				if m.previewBig {
					m.previewBig = false
					return m, nil
				}
				m.closePreview()
				return m, nil
			}
			return m, nil
		case "ctrl+c", "esc", "q":
			if m.previewOpen && key == "esc" {
				if m.previewBig {
					m.previewBig = false
					return m, nil
				}
				m.closePreview()
				return m, nil
			}
			return m, tea.Quit
		case "enter":
			if len(m.visible) == 0 {
				return m, nil
			}
			return m, m.selectRow(m.rows[m.visible[m.cursor]])
		case "up", "k":
			m.moveCursor(-1)
			if m.previewOpen {
				return m, m.openPreviewForSelected()
			}
			return m, nil
		case "down", "j":
			m.moveCursor(1)
			if m.previewOpen {
				return m, m.openPreviewForSelected()
			}
			return m, nil
		case "right":
			cmd := m.openPreviewForSelected()
			m.previewBig = false
			return m, cmd
		case "F", "shift+right":
			if !m.previewOpen {
				cmd := m.openPreviewForSelected()
				m.previewBig = true
				return m, cmd
			}
			m.previewBig = !m.previewBig
			return m, nil
		case "P":
			if !m.previewOpen {
				cmd := m.openPreviewForSelected()
				return m, tea.Batch(cmd, m.previewPopupCmd())
			}
			return m, m.previewPopupCmd()
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
		case "?":
			m.helpOpen = true
			return m, nil
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

func pickerPRRefreshCmd() tea.Cmd {
	return func() tea.Msg {
		_ = refreshPRCaches()
		return pickerPRRefreshMsg{Rows: gatherSessions()}
	}
}

func pickerPRRefreshTickCmd() tea.Cmd {
	return tea.Tick(pickerPRRefreshInterval, func(time.Time) tea.Msg {
		return pickerPRRefreshTickMsg{}
	})
}

func pickerSpinnerCmd() tea.Cmd {
	return tea.Tick(pickerSpinnerInterval, func(time.Time) tea.Msg {
		return pickerSpinnerMsg{}
	})
}

func pickerPreviewRefreshCmd(session, target string) tea.Cmd {
	return func() tea.Msg {
		lines, err := captureTmuxPreview(target)
		return pickerPreviewMsg{Session: session, Target: target, Lines: lines, Err: err}
	}
}

func pickerPreviewTickCmd(session, target string) tea.Cmd {
	return tea.Tick(pickerPreviewInterval, func(time.Time) tea.Msg {
		return pickerPreviewTickMsg{Session: session, Target: target}
	})
}

func windowSizeCmd(width, height int) tea.Cmd {
	return func() tea.Msg {
		return tea.WindowSizeMsg{Width: width, Height: height}
	}
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

func (m *pickerModel) openPreviewForSelected() tea.Cmd {
	session := m.selectedSession()
	if session == nil {
		return nil
	}
	m.previewOpen = true
	m.previewSession = session.Name
	m.previewTarget = preferredPreviewTarget(session.Name)
	m.previewLines = nil
	m.previewErr = nil
	return pickerPreviewRefreshCmd(m.previewSession, m.previewTarget)
}

func (m pickerModel) previewPopupCmd() tea.Cmd {
	if m.previewTarget == "" {
		return nil
	}
	title := " " + m.previewSession
	if pane := previewPaneLabel(m.previewTarget); pane != "" {
		title += " · " + pane
	}
	title += " "
	script := fmt.Sprintf("tmux capture-pane -ep -J -S -5000 -t %s | less -R +G", shellQuote(m.previewTarget))
	cmd := exec.Command("tmux", "display-popup", "-E", "-w", "92%", "-h", "90%", "-T", title, "sh", "-c", script)
	return tea.ExecProcess(cmd, func(err error) tea.Msg {
		return pickerPopupClosedMsg{Err: err}
	})
}

func (m *pickerModel) closePreview() {
	m.previewOpen = false
	m.previewSession = ""
	m.previewTarget = ""
	m.previewLines = nil
	m.previewErr = nil
	m.previewBig = false
}

func (m *pickerModel) clearSelectedSession() tea.Cmd {
	session := m.selectedSession()
	if session == nil {
		return nil
	}
	m.confirmDelete = true
	m.deleteAction = "clear"
	m.deleteSession = session.Name
	m.deleteSlot = 0
	m.deleteRoot = ""
	m.deletePath = session.Path
	m.deleteBranch = ""
	m.deleteForce = false
	m.status = fmt.Sprintf("clear %s? (y/N)", session.Name)
	m.statusErr = false
	return nil
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
		m.deletePath = ""
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
	m.deletePath = ""
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
				row.Session.Codex = ""
				row.Session.ClaudeLabel = ""
				row.Session.CodexLabel = ""
				row.Session.Label = ""
				row.Session.PR = nil
				row.Session.Server = false
				row.Session.Tasks = nil
			}
		}
		m.removeSessionTaskRows(msg.Session)
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

func (m *pickerModel) removeSessionTaskRows(session string) {
	var rows []pickerRow
	for _, row := range m.rows {
		if row.Kind == rowTask && row.Task != nil && row.Task.SessionName == session {
			continue
		}
		rows = append(rows, row)
	}
	m.rows = rows
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

func (m pickerModel) renderConfirmOverlay() string {
	title := "confirm"
	body := pickerActionTarget(pickerActionMsg{
		Action:  m.deleteAction,
		Session: m.deleteSession,
		Value:   m.deleteBranch,
	})
	switch m.deleteAction {
	case "clear":
		title = "clear session"
		body = m.deleteSession + "\n\nclear session state and all todos"
	case "close":
		title = "close session"
		body = m.deleteSession
	case "delete":
		title = "delete workspace"
		body = m.deleteBranch
		if m.deleteForce {
			title = "force delete workspace"
			body = m.deleteBranch + "\n\nuncommitted or unpushed work will be removed"
		}
	}

	titleLine := lipgloss.NewStyle().Foreground(red).Bold(true).Render(title)
	bodyLine := lipgloss.NewStyle().Foreground(text).Render(body)
	hint := lipgloss.NewStyle().Foreground(overlay0).Render("y confirm · any other key cancel")
	box := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(red).
		Padding(1, 2).
		Width(46).
		Render(titleLine + "\n\n" + bodyLine + "\n\n" + hint)
	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, box)
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

func (m pickerModel) renderHelpOverlay() string {
	keyS := lipgloss.NewStyle().Foreground(mauve).Bold(true)
	descS := lipgloss.NewStyle().Foreground(text)
	catS := lipgloss.NewStyle().Foreground(overlay1).Bold(true)
	dim := lipgloss.NewStyle().Foreground(overlay0)
	sep := dim.Render("  ·  ")

	bind := func(k, d string) string { return keyS.Render(k) + descS.Render(" "+d) }
	join := func(parts ...string) string { return strings.Join(parts, sep) }

	var b strings.Builder
	b.WriteString(lipgloss.NewStyle().Foreground(mauve).Bold(true).Render("keybindings") + "\n\n")
	b.WriteString(catS.Render("MOVE") + "\n")
	b.WriteString("  " + join(bind("↑↓ / j k", "move"), bind("g / G", "top / bottom")) + "\n\n")
	b.WriteString(catS.Render("SESSION") + "\n")
	b.WriteString("  " + join(bind("⏎", "switch"), bind("+", "new"), bind("D", "close/delete")) + "\n")
	b.WriteString("  " + join(bind("n", "name"), bind("c", "clear"), bind("r", "reset"), bind("s", "server")) + "\n\n")
	b.WriteString(catS.Render("VIEW") + "\n")
	b.WriteString("  " + join(bind("→", "preview"), bind("F", "big"), bind("P", "popup")) + "\n")
	b.WriteString("  " + join(bind("tab", "show/hide todos"), bind("/", "filter")) + "\n\n")
	b.WriteString(dim.Render("? or esc to close"))

	box := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(mauve).
		Padding(1, 2).
		Render(b.String())
	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, box)
}

func (m pickerModel) View() string {
	if m.width == 0 {
		return ""
	}

	if m.helpOpen {
		return m.renderHelpOverlay()
	}
	if m.confirmDelete {
		return m.renderConfirmOverlay()
	}
	if m.labelEditing {
		return m.renderLabelOverlay()
	}

	var b strings.Builder

	// Interior spans everything between a leading blank line and the
	// trailing blank + footer (plus 1 spare). When a preview is open it
	// claims this full height — logo and list share the left column.
	regionHeight := max(1, m.height-4)
	b.WriteString("\n")
	for _, line := range m.renderInterior(regionHeight) {
		b.WriteString(line + "\n")
	}

	// blank line separating interior from footer
	b.WriteString("\n")

	// Compact footer — most-used actions only; full list lives in the ? overlay.
	sep := pFooterSep.Render(" │ ")
	footer := "    " +
		pFooterKey.Render("↑↓") + pFooter.Render(" navigate") + sep +
		pFooterKey.Render("⏎") + pFooter.Render(" switch") + sep +
		pFooterKey.Render("+") + pFooter.Render(" new") + sep
	if m.previewOpen {
		bigLabel := " big"
		if m.previewBig {
			bigLabel = " shrink"
		}
		footer += pFooterKey.Render("F") + pFooter.Render(bigLabel) + sep +
			pFooterKey.Render("P") + pFooter.Render(" popup") + sep
	}
	footer += pFooterKey.Render("/") + pFooter.Render(" filter") + sep +
		pFooterKey.Render("?") + pFooter.Render(" help") + sep +
		m.renderEscHelp()
	// fit to width minus a right margin matching the 4-col left indent so the
	// footer never bleeds past the right border (also absorbs wide-glyph miscounts)
	b.WriteString(fitANSI(footer, max(1, m.width-4)))

	return b.String()
}

func (m pickerModel) renderEscHelp() string {
	if m.previewBig {
		return pFooterKey.Render("←/esc") + pFooter.Render(" back")
	}
	if m.previewOpen {
		return pFooterKey.Render("←/esc") + pFooter.Render(" preview")
	}
	return pFooterKey.Render("esc") + pFooter.Render(" close")
}

func (m pickerModel) renderContentLines(height int) []string {
	if height < 0 {
		height = 0
	}
	if !m.previewOpen {
		return m.renderListLines(m.width, height)
	}
	if m.previewBig {
		return m.renderPreviewLines(m.width-4, height)
	}
	if m.width < 110 {
		return m.renderPreviewLines(m.width-8, height)
	}
	previewWidth := previewPanelWidth(m.width)
	leftWidth := m.width - previewWidth - 1
	if leftWidth < 24 {
		return m.renderPreviewLines(m.width-8, height)
	}
	left := m.renderListLines(m.width, height)
	right := m.renderPreviewLines(previewWidth, height)
	lines := make([]string, height)
	for i := 0; i < height; i++ {
		lines[i] = fitANSI(left[i], leftWidth) + " " + right[i]
	}
	return lines
}

// renderInterior fills the full vertical region between the leading blank and
// the footer. When a preview is open it spans the whole height; the logo/list
// live in the left column so the preview runs top-to-bottom alongside them.
func (m pickerModel) renderInterior(regionHeight int) []string {
	if regionHeight < 1 {
		regionHeight = 1
	}

	// big preview — full width, full height, no logo
	if m.previewOpen && m.previewBig {
		return padLines(m.renderPreviewLines(m.width-4, regionHeight), regionHeight)
	}

	// split preview — logo + list on the left, full-height preview on the right
	if m.previewOpen && m.width >= 110 {
		previewWidth := previewPanelWidth(m.width)
		leftWidth := m.width - previewWidth - 1
		if leftWidth >= 24 {
			left := m.renderLeftColumn(leftWidth, regionHeight)
			right := padLines(m.renderPreviewLines(previewWidth, regionHeight), regionHeight)
			lines := make([]string, regionHeight)
			for i := 0; i < regionHeight; i++ {
				lines[i] = fitANSI(left[i], leftWidth) + " " + right[i]
			}
			return lines
		}
	}

	// narrow preview — full-width preview, no room for the list beside it
	if m.previewOpen {
		return padLines(m.renderPreviewLines(m.width-8, regionHeight), regionHeight)
	}

	// no preview — logo + filter + list
	return m.renderLeftColumn(m.width, regionHeight)
}

// renderLeftColumn stacks the logo, filter line, and session list into exactly
// regionHeight lines at the given width.
func (m pickerModel) renderLeftColumn(width, regionHeight int) []string {
	lines := m.logoHeaderLines(width)
	lines = append(lines, m.filterLine())
	listHeight := regionHeight - len(lines)
	lines = append(lines, m.renderListLines(width, max(0, listHeight))...)
	return padLines(lines, regionHeight)
}

// logoHeaderLines renders the two-line block-art logo (with optional status
// box right-aligned to width) plus the "switcher" tag.
func (m pickerModel) logoHeaderLines(width int) []string {
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

	out := make([]string, len(logoLines))
	for i, line := range logoLines {
		rendered := "    " + logoStyle.Render(line)
		if i == 0 && statusBox != "" {
			pad := width - lipgloss.Width(rendered) - lipgloss.Width(statusBox) - 4
			if pad < 1 {
				pad = 1
			}
			out[i] = rendered + strings.Repeat(" ", pad) + statusBox
			continue
		}
		if i == 1 {
			rendered += "  " + featureStyle.Render("switcher")
		}
		out[i] = rendered
	}
	return out
}

// filterLine returns the "/ query" prompt when filtering, else a blank line.
func (m pickerModel) filterLine() string {
	if !m.filtering && m.filter.Value() == "" {
		return ""
	}
	line := "    " + lipgloss.NewStyle().Foreground(blue).Render("/") + " " +
		lipgloss.NewStyle().Foreground(text).Render(m.filter.Value())
	if m.filtering {
		line += lipgloss.NewStyle().Foreground(blue).Render("▌")
	}
	return line
}

// padLines truncates or pads lines to exactly n entries.
func padLines(lines []string, n int) []string {
	if len(lines) > n {
		return lines[:n]
	}
	for len(lines) < n {
		lines = append(lines, "")
	}
	return lines
}

func previewPanelWidth(total int) int {
	width := total * 3 / 5
	if width > 80 {
		width = 80
	}
	if width < 44 {
		width = 44
	}
	if width > total-8 {
		width = total - 8
	}
	return width
}

func (m pickerModel) renderListLines(width, height int) []string {
	lines := make([]string, 0, height)
	if height <= 0 {
		return lines
	}
	lm := m
	lm.width = width

	rowLines := make([][]string, 0, len(lm.visible))
	cursorStart, cursorEnd := 0, 0
	lineCount := 0
	for i, vi := range lm.visible {
		rendered := lm.renderRowLines(lm.rows[vi], i == lm.cursor, width)
		rowLines = append(rowLines, rendered)
		if i == lm.cursor {
			cursorStart = lineCount
			cursorEnd = lineCount + len(rendered)
		}
		lineCount += len(rendered)
	}

	startLine := 0
	if cursorEnd > height {
		startLine = cursorEnd - height
	}
	if cursorStart < startLine {
		startLine = cursorStart
	}

	lineIdx := 0
	for _, rendered := range rowLines {
		for _, line := range rendered {
			if lineIdx >= startLine && len(lines) < height {
				lines = append(lines, line)
			}
			lineIdx++
		}
		if len(lines) >= height {
			break
		}
	}
	for len(lines) < height {
		lines = append(lines, "")
	}
	return lines
}

func (m pickerModel) renderRowLines(row pickerRow, selected bool, width int) []string {
	raw := m.renderRow(row, selected)
	parts := strings.Split(raw, "\n")
	lines := make([]string, 0, len(parts))
	for _, part := range parts {
		lines = append(lines, fitANSI(part, width))
	}
	return lines
}

func (m pickerModel) renderPreviewLines(width, height int) []string {
	if height <= 0 {
		return nil
	}
	if width < 24 {
		width = 24
	}
	bodyWidth := max(1, width-4)
	bodyHeight := max(0, height-2)

	title := m.renderPreviewTitle()
	top := roundedTop(title, width)
	bottom := pPreviewRule.Render("╰" + strings.Repeat("─", max(0, width-2)) + "╯")
	lines := []string{top}
	if height == 1 {
		return lines
	}

	var body []string
	bodyStyle := lipgloss.NewStyle()
	styledBody := false
	switch {
	case m.previewErr != nil:
		body = []string{firstLine(m.previewErr.Error())}
		bodyStyle = pPreviewErr
		styledBody = true
	case len(m.previewLines) == 0:
		body = []string{"capturing..."}
		bodyStyle = pPreviewMeta
		styledBody = true
	default:
		body = trimPreviewBlankLines(m.previewLines)
		if len(body) > bodyHeight {
			body = body[len(body)-bodyHeight:]
		}
	}
	for _, line := range body {
		if len(lines) >= height-1 {
			break
		}
		content := fitANSI("    "+line, bodyWidth) + "\x1b[0m"
		if styledBody {
			content = bodyStyle.Render(fitPlain(line, bodyWidth))
		}
		lines = append(lines, pPreviewRule.Render("│")+" "+content+" "+pPreviewRule.Render("│"))
	}
	for len(lines) < height-1 {
		lines = append(lines, pPreviewRule.Render("│")+" "+strings.Repeat(" ", bodyWidth)+" "+pPreviewRule.Render("│"))
	}
	lines = append(lines, bottom)
	return lines
}

func (m pickerModel) renderPreviewTitle() string {
	title := pPreviewTitle.Render("preview " + m.previewSession)
	if session := m.previewSessionInfo(); session != nil {
		if session.Branch != "" {
			title += pSep.Render(" / ") + pPreviewMeta.Render(session.Branch)
		}
		badges := renderAIBadges(session.Claude, session.Codex, session.ClaudeLabel, session.CodexLabel, m.spinnerFrame)
		if badges != "" {
			title += badges
		}
	}
	if pane := previewPaneLabel(m.previewTarget); pane != "" {
		title += pSep.Render(" · ") + pPreviewMeta.Render(pane)
	}
	return title
}

func (m pickerModel) previewSessionInfo() *sessionInfo {
	for _, row := range m.rows {
		if row.Session != nil && row.Session.Name == m.previewSession {
			return row.Session
		}
	}
	return nil
}

func roundedTop(title string, width int) string {
	title = fitANSI(" "+title+" ", max(0, width-4))
	fill := max(0, width-3-lipgloss.Width(title))
	return pPreviewRule.Render("╭─") + title + pPreviewRule.Render(strings.Repeat("─", fill)+"╮")
}

func preferredPreviewTarget(session string) string {
	state := loadSessionStateWithLegacy(session)
	if pane := preferredPreviewPane(state); pane != "" {
		if target, ok := tmuxPaneTarget(session, pane); ok {
			return target
		}
	}
	return session
}

func preferredPreviewPane(state *SessionState) string {
	if state == nil {
		return ""
	}
	bestPane := ""
	bestRank := 0
	bestAgent := 99
	bestValue := ""
	for key, value := range state.AI {
		agent, pane := aiKeyAgentPane(key, value)
		if pane == "" || pane == "default" || pane == "legacy" {
			continue
		}
		if !validTmuxPaneKey(pane) {
			continue
		}
		normalized := normalizeAIState(agent, value)
		rank := aiStateRank(normalized)
		if rank == 0 {
			continue
		}
		agentRank := 1
		if agent == "claude" {
			agentRank = 0
		}
		if rank > bestRank ||
			(rank == bestRank && agentRank < bestAgent) ||
			(rank == bestRank && agentRank == bestAgent && normalized < bestValue) ||
			(rank == bestRank && agentRank == bestAgent && normalized == bestValue && pane < bestPane) {
			bestPane = pane
			bestRank = rank
			bestAgent = agentRank
			bestValue = normalized
		}
	}
	return bestPane
}

func aiKeyAgentPane(key, value string) (string, string) {
	agent, pane, ok := strings.Cut(key, ":")
	if ok {
		agent = strings.ToLower(agent)
		if agent != "claude" && agent != "codex" {
			agent = inferAgentFromAIValue(value)
		}
	} else {
		agent = inferAgentFromAIValue(value)
		pane = key
	}
	if agent == "" {
		agent = "claude"
	}
	return agent, pane
}

func validTmuxPaneKey(key string) bool {
	window, pane, ok := strings.Cut(key, "_")
	if !ok || window == "" || pane == "" {
		return false
	}
	_, err := strconv.Atoi(window)
	if err != nil {
		return false
	}
	_, err = strconv.Atoi(pane)
	return err == nil
}

func tmuxPaneTarget(session, paneKey string) (string, bool) {
	window, pane, ok := strings.Cut(paneKey, "_")
	if !ok {
		return "", false
	}
	return session + ":" + window + "." + pane, true
}

func captureTmuxPreview(target string) ([]string, error) {
	out, err := exec.Command("tmux", "capture-pane", "-ep", "-J", "-S", "-200", "-t", target).CombinedOutput()
	if err != nil {
		msg := strings.TrimSpace(string(out))
		if msg == "" {
			msg = err.Error()
		}
		return nil, fmt.Errorf("tmux capture-pane: %s", msg)
	}
	return previewOutputLines(string(out)), nil
}

func previewOutputLines(out string) []string {
	out = strings.TrimRight(out, "\n")
	if out == "" {
		return nil
	}
	raw := strings.Split(out, "\n")
	lines := make([]string, 0, len(raw))
	for _, line := range raw {
		lines = append(lines, filterPreviewANSI(strings.TrimRight(line, "\r")))
	}
	return lines
}

func trimPreviewBlankLines(lines []string) []string {
	for len(lines) > 0 && strings.TrimSpace(stripANSI(lines[0])) == "" {
		lines = lines[1:]
	}
	for len(lines) > 0 && strings.TrimSpace(stripANSI(lines[len(lines)-1])) == "" {
		lines = lines[:len(lines)-1]
	}
	return lines
}

func filterPreviewANSI(s string) string {
	var b strings.Builder
	for i := 0; i < len(s); {
		if s[i] == '\x1b' {
			if j, final, ok := ansiSequenceEnd(s, i); ok {
				if final == 'm' {
					b.WriteString(s[i:j])
				}
				i = j
				continue
			}
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		if r == utf8.RuneError && size == 1 {
			i++
			continue
		}
		b.WriteRune(r)
		i += size
	}
	return b.String()
}

func stripANSI(s string) string {
	var b strings.Builder
	for i := 0; i < len(s); {
		if s[i] == '\x1b' {
			if j, _, ok := ansiSequenceEnd(s, i); ok {
				i = j
				continue
			}
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		if r == utf8.RuneError && size == 1 {
			i++
			continue
		}
		b.WriteRune(r)
		i += size
	}
	return b.String()
}

func ansiSequenceEnd(s string, i int) (int, byte, bool) {
	if i+1 >= len(s) {
		return len(s), 0, true
	}
	switch s[i+1] {
	case '[':
		for j := i + 2; j < len(s); j++ {
			c := s[j]
			if c >= 0x40 && c <= 0x7e {
				return j + 1, c, true
			}
		}
		return len(s), 0, true
	case ']':
		for j := i + 2; j < len(s); j++ {
			if s[j] == '\a' {
				return j + 1, 0, true
			}
			if s[j] == '\x1b' && j+1 < len(s) && s[j+1] == '\\' {
				return j + 2, 0, true
			}
		}
		return len(s), 0, true
	default:
		return i + 2, 0, true
	}
}

func previewPaneLabel(target string) string {
	_, pane, ok := strings.Cut(target, ":")
	if !ok {
		return ""
	}
	return pane
}

func shellQuote(s string) string {
	if s == "" {
		return "''"
	}
	return "'" + strings.ReplaceAll(s, "'", "'\"'\"'") + "'"
}

func firstLine(s string) string {
	line, _, _ := strings.Cut(s, "\n")
	return strings.TrimSpace(line)
}

func fitPlain(s string, width int) string {
	if width <= 0 {
		return ""
	}
	var b strings.Builder
	used := 0
	for _, r := range s {
		w := lipgloss.Width(string(r))
		if used+w > width {
			break
		}
		b.WriteRune(r)
		used += w
	}
	return b.String()
}

func fitANSI(s string, width int) string {
	if width <= 0 {
		return ""
	}
	var b strings.Builder
	used := 0
	truncated := false
	for i := 0; i < len(s); {
		if s[i] == '\x1b' && i+1 < len(s) && s[i+1] == '[' {
			j := i + 2
			for j < len(s) {
				c := s[j]
				j++
				if c >= 0x40 && c <= 0x7e {
					break
				}
			}
			b.WriteString(s[i:j])
			i = j
			continue
		}
		r, size := utf8.DecodeRuneInString(s[i:])
		if r == utf8.RuneError && size == 1 {
			i++
			continue
		}
		w := lipgloss.Width(string(r))
		if used+w > width {
			truncated = true
			break
		}
		b.WriteRune(r)
		used += w
		i += size
	}
	out := b.String()
	if truncated {
		out += "\x1b[0m"
	}
	if pad := width - lipgloss.Width(out); pad > 0 {
		out += strings.Repeat(" ", pad)
	}
	return out
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

	// First line: {name} on {branch} | {label} ⚡ {AI}
	line.WriteString(nameStyle.Render(s.Name))

	if s.Branch != "" {
		line.WriteString(pBranchDim.Render(" on ") + pBranch.Render(s.Branch))
	}

	var details []string
	if s.Label != "" {
		line.WriteString(pSep.Render(" | ") + labelStyle.Render(s.Label))
	}

	// Server
	if s.Server {
		line.WriteString(" " + pServer.Render("⚡"))
	}

	// AI badge (spinner + working label)
	line.WriteString(renderAIBadges(s.Claude, s.Codex, s.ClaudeLabel, s.CodexLabel, m.spinnerFrame))

	if s.PR != nil {
		pr := fmt.Sprintf("PR#%d", s.PR.Number)
		details = append(details, prStyleForState(s.PR.State).Render(pr))
		if s.PR.Title != "" {
			details = append(details, pSubtitle.Render(s.PR.Title))
		}
	}

	if len(details) > 0 {
		line.WriteString("\n        " + strings.Join(details, pSep.Render(" · ")))
	}

	result := line.String()
	return result
}

func prStyleForState(state string) lipgloss.Style {
	switch state {
	case "OPEN":
		return pPROpen
	case "MERGED":
		return pPRMerged
	case "CLOSED":
		return pPRClosed
	case "DRAFT":
		return pPRDraft
	default:
		return pPROpen
	}
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
	if t.Done {
		marker = pTaskDoneMarker.Render("✓")
		title = pTaskDone.Render(t.Title)
	} else if t.InProgress {
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

		if pr, err := readPRCache(homeDir, sess); err == nil {
			info.PR = pr
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
							Done:        list.Active[i].Done,
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
