package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
	"unicode"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/fsnotify/fsnotify"
)

type model struct {
	path          string
	list          *List
	cursor        int
	cursorArchive bool
	showArchive   bool
	showHelp      bool
	addMode       bool
	editMode      bool
	textInput     textinput.Model
	err           error
	width         int
	height        int
	watcher       *fsnotify.Watcher
	pendingReload bool
}

type fileChangedMsg struct{}
type reloadedMsg struct{ list *List }
type notesEditedMsg struct{ notes string }
type errMsg struct{ err error }

func (e errMsg) Error() string { return e.err.Error() }

const (
	notePadding = 5
	notePrefix  = "│ "
)

// Colors - Catppuccin Mocha palette
var (
	base     = lipgloss.Color("#1e1e2e")
	surface0 = lipgloss.Color("#313244")
	surface1 = lipgloss.Color("#45475a")
	overlay0 = lipgloss.Color("#6c7086")
	overlay1 = lipgloss.Color("#7f849c")
	subtext0 = lipgloss.Color("#a6adc8")
	text     = lipgloss.Color("#cdd6f4")
	blue     = lipgloss.Color("#89b4fa")
	green    = lipgloss.Color("#a6e3a1")
	yellow   = lipgloss.Color("#f9e2af")
	peach    = lipgloss.Color("#fab387")
	red      = lipgloss.Color("#f38ba8")
	mauve    = lipgloss.Color("#cba6f7")
	flamingo = lipgloss.Color("#f2cdcd")
	pink     = lipgloss.Color("#E3B4D8")
	teal     = lipgloss.Color("#93E2D5")

	compactPurple = lipgloss.Color("#AFAFFF")
)

// Styles
var (
	headerStyle = lipgloss.NewStyle().
			Foreground(blue).
			Bold(true)

	worktreeStyle = lipgloss.NewStyle().
			Foreground(subtext0)

	taskCountStyle = lipgloss.NewStyle().
			Foreground(overlay1)

	cursorStyle = lipgloss.NewStyle().
			Foreground(blue).
			Bold(true)

	selectedRowStyle = lipgloss.NewStyle().
				Padding(0, 1)

	normalRowStyle = lipgloss.NewStyle().
			Padding(0, 1)

	checkboxEmpty = lipgloss.NewStyle().
			Foreground(overlay0)

	checkboxDone = lipgloss.NewStyle().
			Foreground(green)

	checkboxProgress = lipgloss.NewStyle().
				Foreground(yellow)

	taskTitleStyle = lipgloss.NewStyle().
			Foreground(text)

	taskTitleSelectedStyle = lipgloss.NewStyle().
				Foreground(text).
				Bold(true)

	taskTitleProgressStyle = lipgloss.NewStyle().
				Foreground(yellow)

	taskTitleProgressSelectedStyle = lipgloss.NewStyle().
					Foreground(yellow).
					Bold(true)

	notesStyle = lipgloss.NewStyle().
			Foreground(overlay1).
			Italic(true).
			PaddingLeft(notePadding)

	archiveHeaderStyle = lipgloss.NewStyle().
				Foreground(overlay1).
				Bold(true)

	archiveRuleStyle = lipgloss.NewStyle().
				Foreground(surface1)

	archiveTitleStyle = lipgloss.NewStyle().
				Foreground(overlay0)

	archiveTitleSelectedStyle = lipgloss.NewStyle().
					Foreground(text).
					Bold(true)

	archiveDateStyle = lipgloss.NewStyle().
				Foreground(surface1)

	archiveDateSelectedStyle = lipgloss.NewStyle().
					Foreground(subtext0)

	footerStyle = lipgloss.NewStyle().
			Foreground(overlay0)

	footerKeyStyle = lipgloss.NewStyle().
			Foreground(blue).
			Bold(true)

	footerDescStyle = lipgloss.NewStyle().
			Foreground(overlay0)

	footerSepStyle = lipgloss.NewStyle().
			Foreground(surface1)

	inputStyle = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(blue).
			Padding(0, 1).
			MarginTop(1)

	helpStyle = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(surface1).
			Padding(1, 2)

	helpKeyStyle = lipgloss.NewStyle().
			Foreground(blue).
			Bold(true).
			Width(16)

	helpDescStyle = lipgloss.NewStyle().
			Foreground(subtext0)

	emptyStyle = lipgloss.NewStyle().
			Foreground(overlay0).
			Italic(true).
			PaddingLeft(2)

	separatorStyle = lipgloss.NewStyle().
			Foreground(surface1)
)

// shortenPath shows just the project name + branch/worktree suffix
func shortenPath(p string) string {
	home, _ := os.UserHomeDir()
	if home != "" && strings.HasPrefix(p, home) {
		p = "~" + p[len(home):]
	}

	// For known scratch worktrees, show "bar (workspace-1)".
	if idx, ok := knownWorkspacePathIndex(p); ok {
		project := filepath.Base(p[:idx])
		wt := filepath.Base(p)
		return project + " (" + wt + ")"
	}
	if idx := strings.Index(p, "/.git/worktrees/"); idx >= 0 {
		project := filepath.Base(p[:idx])
		wt := filepath.Base(p)
		return project + " (" + wt + ")"
	}

	// For regular repos, just show the last 2 path components
	parts := strings.Split(p, "/")
	if len(parts) > 2 {
		return strings.Join(parts[len(parts)-2:], "/")
	}
	return p
}

func runTUI(path string) error {
	list, err := loadList(path)
	if err != nil {
		return fmt.Errorf("cannot load list: %w", err)
	}

	if list.Worktree == "" {
		base := filepath.Base(path)
		base = strings.TrimSuffix(base, ".md")
		list.Worktree = strings.ReplaceAll(base, "%2F", "/")
	}

	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return fmt.Errorf("cannot create file watcher: %w", err)
	}
	defer watcher.Close()

	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("cannot create storage dir: %w", err)
	}
	_ = watcher.Add(dir)

	ti := textinput.New()
	ti.Placeholder = "what needs doing?"
	ti.CharLimit = 200
	ti.PromptStyle = lipgloss.NewStyle().Foreground(blue)
	ti.TextStyle = lipgloss.NewStyle().Foreground(text)

	m := model{
		path:      path,
		list:      list,
		textInput: ti,
		watcher:   watcher,
	}

	p := tea.NewProgram(m, tea.WithAltScreen())

	targetBase := filepath.Base(path)
	go func() {
		for {
			select {
			case event, ok := <-watcher.Events:
				if !ok {
					return
				}
				if filepath.Base(event.Name) == targetBase &&
					(event.Op&fsnotify.Write != 0 || event.Op&fsnotify.Create != 0) {
					p.Send(fileChangedMsg{})
				}
			case _, ok := <-watcher.Errors:
				if !ok {
					return
				}
			}
		}
	}()

	_, err = p.Run()
	return err
}

func (m model) Init() tea.Cmd {
	return nil
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil

	case fileChangedMsg:
		if !m.addMode && !m.editMode {
			return m, m.reloadList()
		}
		m.pendingReload = true
		return m, nil

	case reloadedMsg:
		m.list = msg.list
		m.pendingReload = false
		m.clampCursor()
		return m, nil

	case notesEditedMsg:
		if idx, ok := m.selectedActiveIndex(); ok {
			m.list.Active[idx].Notes = msg.notes
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
		}
		return m, nil

	case errMsg:
		m.err = msg.err
		return m, nil

	case tea.KeyMsg:
		return m.handleKey(msg)
	}

	var cmd tea.Cmd
	if m.addMode || m.editMode {
		m.textInput, cmd = m.textInput.Update(msg)
	}
	return m, cmd
}

func (m model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	if m.showHelp {
		switch msg.String() {
		case "?", "q", "enter":
			m.showHelp = false
		}
		return m, nil
	}

	if m.addMode || m.editMode {
		switch msg.String() {
		case "enter":
			title := strings.TrimSpace(m.textInput.Value())
			if m.addMode && title != "" {
				m.list.Active = append(m.list.Active, Item{Title: title})
				if err := saveList(m.path, m.list); err != nil {
					m.err = err
				}
			}
			if m.editMode && title != "" {
				if idx, ok := m.selectedActiveIndex(); ok {
					m.list.Active[idx].Title = title
				} else if idx, ok := m.selectedArchiveIndex(); ok {
					m.list.Archive[idx].Title = title
				}
				if err := saveList(m.path, m.list); err != nil {
					m.err = err
				}
			}
			m.addMode = false
			m.editMode = false
			m.textInput.SetValue("")
			if m.pendingReload {
				return m, m.reloadList()
			}
			return m, nil
		case "esc":
			m.addMode = false
			m.editMode = false
			m.textInput.SetValue("")
			if m.pendingReload {
				return m, m.reloadList()
			}
			return m, nil
		}
		var cmd tea.Cmd
		m.textInput, cmd = m.textInput.Update(msg)
		return m, cmd
	}

	switch msg.String() {
	case "q", "esc", "ctrl+c":
		return m, tea.Quit

	case "?":
		m.showHelp = true
		return m, nil

	case "j", "down":
		m.moveCursor(1)

	case "k", "up":
		m.moveCursor(-1)

	case "g":
		m.setFlatCursor(0)

	case "G":
		if count := m.selectableCount(); count > 0 {
			m.setFlatCursor(count - 1)
		}

	case "a":
		m.addMode = true
		m.editMode = false
		m.textInput.SetValue("")
		m.textInput.Focus()
		return m, textinput.Blink

	case "e":
		if title, ok := m.selectedTitle(); ok {
			m.addMode = false
			m.editMode = true
			m.textInput.SetValue(title)
			m.textInput.Focus()
			return m, textinput.Blink
		}

	case "i":
		if idx, ok := m.selectedActiveIndex(); ok {
			m.list.Active[idx].InProgress = !m.list.Active[idx].InProgress
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
		} else if idx, ok := m.selectedArchiveIndex(); ok {
			m.restoreArchivedItem(idx, true)
		}

	case "o":
		if idx, ok := m.selectedArchiveIndex(); ok {
			m.restoreArchivedItem(idx, false)
		}

	case "f":
		if idx, ok := m.selectedActiveIndex(); ok {
			ensureItemID(&m.list.Active[idx])
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
				return m, nil
			}
			if session, err := currentTmuxSession(); err == nil {
				state := loadSessionStateWithLegacy(session)
				state.FocusedTodoID = m.list.Active[idx].ID
				if state.TodoPath == "" {
					state.TodoPath = m.path
				}
				if err := saveSessionState(state); err != nil {
					m.err = err
				}
			}
		}

	case "x", " ":
		if idx, ok := m.selectedActiveIndex(); ok {
			item := m.list.Active[idx]
			item.InProgress = false
			item.Done = true
			item.DoneDate = time.Now().Format("2006-01-02")
			m.list.Archive = append([]Item{item}, m.list.Archive...)
			m.list.Active = append(m.list.Active[:idx], m.list.Active[idx+1:]...)
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
			m.clampCursor()
		}

	case "d":
		if idx, ok := m.selectedActiveIndex(); ok {
			m.list.Active = append(m.list.Active[:idx], m.list.Active[idx+1:]...)
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
			m.clampCursor()
		} else if idx, ok := m.selectedArchiveIndex(); ok {
			m.list.Archive = append(m.list.Archive[:idx], m.list.Archive[idx+1:]...)
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
			m.clampCursor()
		}

	case "J":
		if idx, ok := m.selectedActiveIndex(); ok && idx < len(m.list.Active)-1 {
			m.list.Active[idx], m.list.Active[idx+1] = m.list.Active[idx+1], m.list.Active[idx]
			m.cursor++
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
		}

	case "K":
		if idx, ok := m.selectedActiveIndex(); ok && idx > 0 {
			m.list.Active[idx], m.list.Active[idx-1] = m.list.Active[idx-1], m.list.Active[idx]
			m.cursor--
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
		}

	case "D":
		if m.showArchive && len(m.list.Archive) > 0 {
			m.list.Archive = nil
			if err := saveList(m.path, m.list); err != nil {
				m.err = err
			}
			m.clampCursor()
		}

	case "tab":
		m.showArchive = !m.showArchive
		m.clampCursor()

	case "r":
		return m, m.reloadList()

	case "enter":
		if _, ok := m.selectedActiveIndex(); ok {
			return m, m.openEditor()
		}
	}

	return m, nil
}

func (m model) selectableCount() int {
	count := len(m.list.Active)
	if m.showArchive {
		count += len(m.list.Archive)
	}
	return count
}

func (m model) flatCursor() int {
	if m.cursorArchive {
		return len(m.list.Active) + m.cursor
	}
	return m.cursor
}

func (m *model) setFlatCursor(pos int) {
	count := m.selectableCount()
	if count == 0 {
		m.cursorArchive = false
		m.cursor = 0
		return
	}
	if pos < 0 {
		pos = 0
	}
	if pos >= count {
		pos = count - 1
	}

	activeCount := len(m.list.Active)
	if pos < activeCount {
		m.cursorArchive = false
		m.cursor = pos
		return
	}

	m.cursorArchive = true
	m.cursor = pos - activeCount
}

func (m *model) moveCursor(dir int) {
	count := m.selectableCount()
	if count == 0 {
		m.cursorArchive = false
		m.cursor = 0
		return
	}
	m.setFlatCursor(m.flatCursor() + dir)
}

func (m *model) clampCursor() {
	if m.selectableCount() == 0 {
		m.cursorArchive = false
		m.cursor = 0
		return
	}
	m.setFlatCursor(m.flatCursor())
}

func (m model) selectedActiveIndex() (int, bool) {
	if m.cursorArchive || m.cursor < 0 || m.cursor >= len(m.list.Active) {
		return 0, false
	}
	return m.cursor, true
}

func (m model) selectedArchiveIndex() (int, bool) {
	if !m.cursorArchive || !m.showArchive || m.cursor < 0 || m.cursor >= len(m.list.Archive) {
		return 0, false
	}
	return m.cursor, true
}

func (m model) selectedTitle() (string, bool) {
	if idx, ok := m.selectedActiveIndex(); ok {
		return m.list.Active[idx].Title, true
	}
	if idx, ok := m.selectedArchiveIndex(); ok {
		return m.list.Archive[idx].Title, true
	}
	return "", false
}

func (m *model) restoreArchivedItem(idx int, inProgress bool) {
	if idx < 0 || idx >= len(m.list.Archive) {
		return
	}
	item := m.list.Archive[idx]
	item.Done = false
	item.DoneDate = ""
	item.InProgress = inProgress
	m.list.Archive = append(m.list.Archive[:idx], m.list.Archive[idx+1:]...)
	m.list.Active = append([]Item{item}, m.list.Active...)
	m.cursorArchive = false
	m.cursor = 0
	if err := saveList(m.path, m.list); err != nil {
		m.err = err
	}
}

func (m model) View() string {
	if m.err != nil {
		errBox := lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(red).
			Padding(1, 2).
			Render(fmt.Sprintf("Error: %v\n\nPress q to quit.", m.err))
		return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, errBox)
	}

	if m.showHelp {
		return m.helpView()
	}

	if m.addMode || m.editMode {
		return m.renderAddOverlay()
	}

	var sections []string

	const indent = "    "

	// Header — block logo matching session picker
	logoStyle := lipgloss.NewStyle().Foreground(mauve).Bold(true)
	logoLines := []string{
		"█▀▄ █▀█ █▀▄▀█ █ █ ▀▄▀",
		"█▄▀ █▄█ █ ▀ █ █▄█ █ █",
	}
	count := taskCountStyle.Render(fmt.Sprintf("%d tasks", len(m.list.Active)))
	worktree := worktreeStyle.Render(shortenPath(m.list.Worktree))
	innerWidth := m.width - 8 // 4-space indent on each side
	if innerWidth < 20 {
		innerWidth = 40
	}

	// First logo line with count right-aligned
	firstLine := logoStyle.Render(logoLines[0])
	pad := innerWidth - lipgloss.Width(firstLine) - lipgloss.Width(count)
	if pad < 1 {
		pad = 1
	}
	sections = append(sections, "\n"+indent+firstLine+strings.Repeat(" ", pad)+count)
	featureStyle := lipgloss.NewStyle().Foreground(overlay1).Italic(true)
	sections = append(sections, indent+logoStyle.Render(logoLines[1])+"  "+featureStyle.Render("todo"))
	sections = append(sections, "")
	sections = append(sections, indent+worktree)
	sections = append(sections, indent+separatorStyle.Render(strings.Repeat("─", innerWidth)))

	// Task list
	if len(m.list.Active) == 0 {
		sections = append(sections, indent+emptyStyle.Render("no tasks — press a to add"))
	} else {
		var tasks []string
		for i, item := range m.list.Active {
			selected := !m.cursorArchive && i == m.cursor

			var row string
			checkbox := checkboxEmpty.Render("○")
			title := taskTitleStyle.Render(item.Title)
			if item.InProgress {
				checkbox = checkboxProgress.Render("●")
				title = taskTitleProgressStyle.Render(item.Title)
			}
			if selected {
				if item.InProgress {
					title = taskTitleProgressSelectedStyle.Render(item.Title)
				} else {
					title = taskTitleSelectedStyle.Render(item.Title)
				}
				content := cursorStyle.Render("›") + " " + checkbox + " " + title
				row = indent + selectedRowStyle.Render(content)
			} else {
				row = indent + normalRowStyle.Render("  "+checkbox+" "+title)
			}
			tasks = append(tasks, row)

			if item.Notes != "" {
				for _, line := range wrapNoteLines(item.Notes, innerWidth) {
					tasks = append(tasks, indent+notesStyle.Render(notePrefix+line))
				}
			}
		}
		sections = append(sections, strings.Join(tasks, "\n"))
	}

	// Archive — group header with rule, matching session picker style
	archiveCount := len(m.list.Archive)
	if archiveCount > 0 {
		label := archiveHeaderStyle.Render(fmt.Sprintf("ARCHIVE (%d)", archiveCount))
		toggle := archiveRuleStyle.Render(" ▾")
		if !m.showArchive {
			toggle = archiveRuleStyle.Render(" ▸")
		}
		ruleWidth := innerWidth - lipgloss.Width(label) - lipgloss.Width(toggle) - 2
		if ruleWidth < 1 {
			ruleWidth = 1
		}
		rule := "  " + archiveRuleStyle.Render(strings.Repeat("─", ruleWidth))
		archHeader := "\n" + indent + label + toggle + rule
		if m.showArchive {
			var archiveLines []string
			archiveLines = append(archiveLines, archHeader)
			for i, item := range m.list.Archive {
				selected := m.cursorArchive && i == m.cursor
				date := archiveDateStyle.Render(item.DoneDate)
				title := archiveTitleStyle.Render(item.Title)
				check := checkboxDone.Render("✓")
				if selected {
					date = archiveDateSelectedStyle.Render(item.DoneDate)
					title = archiveTitleSelectedStyle.Render(item.Title)
					content := cursorStyle.Render("›") + " " + check + " " + date + "  " + title
					archiveLines = append(archiveLines, indent+selectedRowStyle.Render(content))
				} else {
					archiveLines = append(archiveLines, fmt.Sprintf(indent+"  %s %s  %s", check, date, title))
				}
			}
			sections = append(sections, strings.Join(archiveLines, "\n"))
		} else {
			sections = append(sections, archHeader)
		}
	}

	sections = append(sections, "")
	sections = append(sections, indent+m.renderFooter())

	return lipgloss.NewStyle().PaddingTop(1).Render(
		lipgloss.JoinVertical(lipgloss.Left, sections...),
	)
}

func (m model) renderAddOverlay() string {
	innerWidth := 60
	if m.width > 0 && m.width-12 < innerWidth {
		innerWidth = m.width - 12
		if innerWidth < 24 {
			innerWidth = 24
		}
	}

	title := "add task"
	action := "add"
	if m.editMode {
		title = "edit task"
		action = "save"
	}

	titleLine := lipgloss.NewStyle().Foreground(blue).Bold(true).Render(title)
	inputLine := m.textInput.View()
	hint := lipgloss.NewStyle().Foreground(overlay0).Render("enter " + action + " · esc cancel")

	box := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(blue).
		Padding(1, 2).
		Width(innerWidth).
		Render(titleLine + "\n\n" + inputLine + "\n\n" + hint)
	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, box)
}

func wrapNoteLines(notes string, innerWidth int) []string {
	width := innerWidth - notePadding - lipgloss.Width(notePrefix)
	if width < 1 {
		width = 1
	}

	var lines []string
	for _, line := range strings.Split(notes, "\n") {
		lines = append(lines, wrapLine(line, width)...)
	}
	return lines
}

func wrapLine(line string, width int) []string {
	if line == "" {
		return []string{""}
	}

	var lines []string
	for lipgloss.Width(line) > width {
		cut := fitWidthIndex(line, width)
		if cut >= len(line) {
			break
		}

		if breakAt := lastSpaceIndex(line[:cut]); breakAt > 0 {
			lines = append(lines, strings.TrimRightFunc(line[:breakAt], unicode.IsSpace))
			line = strings.TrimLeftFunc(line[breakAt:], unicode.IsSpace)
			if line == "" {
				return lines
			}
			continue
		}

		lines = append(lines, line[:cut])
		line = strings.TrimLeftFunc(line[cut:], unicode.IsSpace)
		if line == "" {
			return lines
		}
	}

	return append(lines, line)
}

func fitWidthIndex(s string, width int) int {
	end := 0
	for i := range s {
		if i == 0 {
			continue
		}
		if lipgloss.Width(s[:i]) > width {
			if end == 0 {
				return i
			}
			return end
		}
		end = i
	}
	if lipgloss.Width(s) > width && end > 0 {
		return end
	}
	return len(s)
}

func lastSpaceIndex(s string) int {
	last := -1
	for i, r := range s {
		if unicode.IsSpace(r) {
			last = i
		}
	}
	return last
}

func (m model) renderFooter() string {
	keys := []struct{ key, desc string }{
		{"a", "add"},
		{"e", "edit"},
		{"i", "progress"},
		{"f", "focus"},
		{"o", "open"},
		{"x", "done"},
		{"⏎", "notes"},
		{"d", "del"},
		{"J/K", "move"},
		{"⇥", "archive"},
		{"?", "help"},
		{"q", "quit"},
	}

	sep := footerSepStyle.Render(" │ ")
	var parts []string
	for _, k := range keys {
		part := footerKeyStyle.Render(k.key) + " " + footerDescStyle.Render(k.desc)
		parts = append(parts, part)
	}
	return footerStyle.Render(strings.Join(parts, sep))
}

func (m model) helpView() string {
	type binding struct{ key, desc string }
	bindings := []binding{
		{"j / k", "move cursor"},
		{"g / G", "top / bottom"},
		{"a", "add new task"},
		{"e", "edit selected task"},
		{"i", "toggle / restore in progress"},
		{"f", "focus selected task for session"},
		{"o", "reopen archived task"},
		{"enter", "edit notes in $EDITOR"},
		{"space / x", "mark done → archive"},
		{"d", "delete task"},
		{"J / K", "reorder task up/down"},
		{"tab", "toggle archive"},
		{"r", "reload from disk"},
		{"?", "toggle help"},
		{"q / ctrl+c", "quit"},
	}

	var lines []string
	for _, b := range bindings {
		line := helpKeyStyle.Render(b.key) + helpDescStyle.Render(b.desc)
		lines = append(lines, line)
	}

	title := lipgloss.NewStyle().Foreground(blue).Bold(true).Render("keybindings")
	content := title + "\n\n" + strings.Join(lines, "\n") + "\n\n" +
		footerDescStyle.Render("press ? or q to close")

	box := helpStyle.Render(content)
	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, box)
}

func (m model) reloadList() tea.Cmd {
	return func() tea.Msg {
		list, err := loadList(m.path)
		if err != nil {
			return errMsg{err}
		}
		return reloadedMsg{list}
	}
}

func (m model) openEditor() tea.Cmd {
	idx, ok := m.selectedActiveIndex()
	if !ok {
		return nil
	}
	item := m.list.Active[idx]

	tmpfile, err := os.CreateTemp("", "domux-notes-*.txt")
	if err != nil {
		return func() tea.Msg { return errMsg{err} }
	}
	tmpfile.WriteString(item.Notes)
	tmpfile.Close()

	editor := os.Getenv("EDITOR")
	if editor == "" {
		editor = "vi"
	}
	c := exec.Command(editor, tmpfile.Name())

	return tea.ExecProcess(c, func(err error) tea.Msg {
		defer os.Remove(tmpfile.Name())
		if err != nil {
			return errMsg{err}
		}
		content, err := os.ReadFile(tmpfile.Name())
		if err != nil {
			return errMsg{err}
		}
		return notesEditedMsg{strings.TrimSpace(string(content))}
	})
}
