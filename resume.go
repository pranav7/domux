package main

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
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
// a workspace-N worktree groups under its main checkout.
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
