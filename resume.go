package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
)

type resumeStatus string

const (
	resumeRecreated resumeStatus = "recreated"
	resumeRunning   resumeStatus = "already running"
	resumePruned    resumeStatus = "pruned"
)

// resumeTarget is one saved session to act on. Prune is set by planResume when
// the session's Root no longer exists on disk; Status/Err/nWindows/nAgents are
// filled by executeResumeStep.
type resumeTarget struct {
	Name     string
	Root     string
	Group    string
	Prune    bool
	Status   resumeStatus
	Err      error
	nWindows int
	nAgents  int
}

// resumeGroup derives the picker-group key for a root, mirroring gatherSessions:
// a workspace-N worktree groups under its main checkout. Unlike gatherSessions
// it does not git-normalize first — it assumes root is already a git toplevel,
// which setSessionRoot guarantees for every saved state. If a future change
// ever pins a subdirectory as Root, this would need the same `git rev-parse
// --show-toplevel` step gatherSessions uses, or group filtering would miss.
func resumeGroup(root string) string {
	if main, ok := workspaceRootFromPath(root); ok {
		return filepath.Base(main)
	}
	return filepath.Base(root)
}

// planResume partitions saved states into sessions to recreate (Root exists)
// and dead state files to prune (Root gone). When filter != "", only states
// whose group equals filter (case-insensitive) are considered.
func planResume(states []SessionState, filter string) (recreate, prune []resumeTarget) {
	for _, s := range states {
		if s.Name == "" || s.Root == "" {
			continue
		}
		group := resumeGroup(s.Root)
		if filter != "" && !strings.EqualFold(group, filter) {
			continue
		}
		t := resumeTarget{Name: s.Name, Root: s.Root, Group: group}
		if dirExists(s.Root) {
			recreate = append(recreate, t)
		} else {
			t.Prune = true
			prune = append(prune, t)
		}
	}
	sortResumeTargets(recreate)
	sortResumeTargets(prune)
	return recreate, prune
}

// sortResumeTargets orders by group then name, matching the picker's row order.
func sortResumeTargets(ts []resumeTarget) {
	sort.Slice(ts, func(i, j int) bool {
		if ts[i].Group != ts[j].Group {
			return ts[i].Group < ts[j].Group
		}
		return ts[i].Name < ts[j].Name
	})
}

// availableResumeGroups returns the sorted unique group names across saved
// states — used to hint the user when a project filter matches nothing.
func availableResumeGroups(states []SessionState) []string {
	set := map[string]bool{}
	for _, s := range states {
		if s.Name == "" || s.Root == "" {
			continue
		}
		set[resumeGroup(s.Root)] = true
	}
	groups := make([]string, 0, len(set))
	for g := range set {
		groups = append(groups, g)
	}
	sort.Strings(groups)
	return groups
}

// executeResumeStep performs one target's side effect: prune removes its state
// files; recreate creates a detached tmux session at the saved root (skipping
// sessions tmux already has), replays its saved window layout, relaunches the
// agent that ran in each window, and re-pins Root/TodoPath. Errors are captured
// on the returned target, not propagated — the caller tallies them and continues.
func executeResumeStep(t resumeTarget) resumeTarget {
	if t.Prune {
		if home, err := os.UserHomeDir(); err == nil {
			_ = clearSessionStateFiles(home, t.Name)
		}
		if err := removeSessionState(t.Name); err != nil {
			t.Err = err
		}
		t.Status = resumePruned
		return t
	}
	if tmuxSessionExists(t.Name) {
		t.Status = resumeRunning
		return t
	}
	// Read the saved window layout BEFORE createTmuxSession — the fresh session
	// has one default window, and the setSessionRoot save below re-snapshots the
	// live layout, so we must recreate windows first or the good snapshot is lost.
	savedWindows := loadSessionStateWithLegacy(t.Name).Windows
	if err := createTmuxSession(t.Name, t.Root); err != nil {
		t.Err = err
		return t
	}
	t.nWindows, t.nAgents = resumeWindows(t.Name, savedWindows)
	// setSessionRoot's save now re-snapshots the recreated layout, keeping the
	// persisted Windows in sync with what we just rebuilt.
	if _, err := setSessionRoot(t.Name, t.Root); err != nil {
		t.Err = err
		return t
	}
	t.Status = resumeRecreated
	return t
}

// resumeWindows replays a saved window layout into an already-created session
// and relaunches the agent that last ran in each window. Returns the number of
// windows recreated and agents relaunched. Best-effort: a tmux error on any one
// window is skipped, not fatal — a partially-restored session beats none.
func resumeWindows(session string, windows []WindowSnapshot) (nWindows, nAgents int) {
	if len(windows) == 0 {
		return 0, 0
	}
	// The lone default window becomes the first snapshot; the rest are created.
	// Rename by session target (the sole/active window) so we don't guess at
	// tmux's base-index.
	_ = exec.Command("tmux", "rename-window", "-t", session, windows[0].Name).Run()
	for _, w := range windows[1:] {
		_ = exec.Command("tmux", newWindowArgs(session, w.Name, w.Cwd)...).Run()
	}

	// Real tmux indices, in creation order, to target send-keys precisely — the
	// active window after the new-window calls is the last one, not the first.
	indices := tmuxWindowIndices(session)
	nWindows = len(windows)

	sessions := readAgentSessions()
	for i, w := range windows {
		if i >= len(indices) {
			break
		}
		as, ok := bestAgentSession(sessions, w.Cwd)
		if !ok {
			continue
		}
		line := resumeAgentLaunchLine(as.Agent, as.SessionID, w.Cwd)
		if line == "" {
			continue
		}
		target := fmt.Sprintf("%s:%d", session, indices[i])
		if exec.Command("tmux", "send-keys", "-t", target, line, "Enter").Run() == nil {
			nAgents++
		}
	}
	return nWindows, nAgents
}

// tmuxWindowIndices returns a session's window indices in tmux's own order
// (ascending by index), used to map saved windows to the ones just recreated.
func tmuxWindowIndices(session string) []int {
	out, err := exec.Command("tmux", "list-windows", "-t", session, "-F", "#{window_index}").Output()
	if err != nil {
		return nil
	}
	var idxs []int
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		if line == "" {
			continue
		}
		if n, err := strconv.Atoi(line); err == nil {
			idxs = append(idxs, n)
		}
	}
	return idxs
}

// resumeAgentLaunchLine builds the shell line sent into a window to resume an
// agent conversation. It cds into the window's cwd first (claude --resume is
// scoped to the project dir) and guards on the binary existing, so a missing
// CLI or gone directory no-ops cleanly instead of erroring in the pane.
func resumeAgentLaunchLine(agent, sessionID, cwd string) string {
	var cmd string
	switch agent {
	case "claude":
		cmd = "claude --resume " + shellQuote(sessionID)
	case "codex":
		cmd = "codex resume " + shellQuote(sessionID)
	case "opencode":
		cmd = "opencode --session " + shellQuote(sessionID)
	default:
		return ""
	}
	guarded := "command -v " + agent + " >/dev/null 2>&1 && " + cmd
	if cwd != "" {
		return "cd " + shellQuote(cwd) + " && " + guarded
	}
	return guarded
}

// resumeJob tracks sequential progress through a queue of resume targets
// (prune entries first, then recreate). One step runs at a time; record()
// advances the cursor and tallies the outcome.
type resumeJob struct {
	queue      []resumeTarget
	pos        int
	done       bool
	nRecreated int
	nRunning   int
	nPruned    int
	nFailed    int
	nWindows   int
	nAgents    int
}

func (j *resumeJob) nextTarget() (resumeTarget, bool) {
	if j.pos >= len(j.queue) {
		return resumeTarget{}, false
	}
	return j.queue[j.pos], true
}

func (j *resumeJob) record(t resumeTarget) {
	switch {
	case t.Err != nil:
		j.nFailed++
	case t.Status == resumeRecreated:
		j.nRecreated++
	case t.Status == resumeRunning:
		j.nRunning++
	case t.Status == resumePruned:
		j.nPruned++
	}
	j.nWindows += t.nWindows
	j.nAgents += t.nAgents
	j.pos++
}

func (j *resumeJob) summary() string {
	s := fmt.Sprintf("restored %d", j.nRecreated)
	if j.nWindows > 0 {
		s += fmt.Sprintf(" (%d windows, %d agents resumed)", j.nWindows, j.nAgents)
	}
	s += fmt.Sprintf(" · running %d · pruned %d", j.nRunning, j.nPruned)
	if j.nFailed > 0 {
		s += fmt.Sprintf(" · %d failed", j.nFailed)
	}
	return s
}

type resumeStepMsg struct {
	target resumeTarget
}

// resumeStepCmd runs one resume step off the bubbletea event loop and reports
// the result back as a resumeStepMsg.
func resumeStepCmd(t resumeTarget) tea.Cmd {
	return func() tea.Msg {
		return resumeStepMsg{target: executeResumeStep(t)}
	}
}
