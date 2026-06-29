package main

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
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
// the session's Root no longer exists on disk; Status/Err are filled by
// executeResumeStep.
type resumeTarget struct {
	Name   string
	Root   string
	Group  string
	Prune  bool
	Status resumeStatus
	Err    error
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
// sessions tmux already has) and re-pins Root/TodoPath. Errors are captured on
// the returned target, not propagated — the caller tallies them and continues.
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
	if err := createTmuxSession(t.Name, t.Root); err != nil {
		t.Err = err
		return t
	}
	if _, err := setSessionRoot(t.Name, t.Root); err != nil {
		t.Err = err
		return t
	}
	t.Status = resumeRecreated
	return t
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
	j.pos++
}

func (j *resumeJob) summary() string {
	s := fmt.Sprintf("restored %d · running %d · pruned %d", j.nRecreated, j.nRunning, j.nPruned)
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
