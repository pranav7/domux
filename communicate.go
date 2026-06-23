package main

import (
	"fmt"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

// commTarget identifies a peer agent's tmux pane resolved from a worktree name.
type commTarget struct {
	Name    string // the name the caller passed
	Session string // resolved tmux session
	Root    string // worktree root (may be "")
	Label   string // human label, if any
	Pane    string // "1.0"
	Target  string // "workspace-2:1.0"
	Command string // pane_current_command at resolve time (best-effort)
}

// tmuxPane is one row from `tmux list-panes`.
type tmuxPane struct {
	Spec    string // "1.0" (per-session) or "session:1.0" (-a form)
	Command string
	Title   string
	Path    string
}

var (
	paneSpecRe      = regexp.MustCompile(`^\d+[._]\d+$`)
	claudeVersionRe = regexp.MustCompile(`^\d+(\.\d+)+$`)
)

// matchSessionsByName returns states whose name, worktree dir basename, or
// label equal name (case-insensitive).
func matchSessionsByName(name string, states []SessionState) []SessionState {
	want := strings.ToLower(strings.TrimSpace(name))
	if want == "" {
		return nil
	}
	var matches []SessionState
	for _, s := range states {
		if sessionMatchesName(want, s) {
			matches = append(matches, s)
		}
	}
	return matches
}

func sessionMatchesName(want string, s SessionState) bool {
	if strings.ToLower(s.Name) == want {
		return true
	}
	if s.Root != "" && strings.ToLower(filepath.Base(s.Root)) == want {
		return true
	}
	if s.Label != "" && strings.ToLower(strings.TrimSpace(s.Label)) == want {
		return true
	}
	return false
}

// normalizePaneSpec accepts "1_0" or "1.0" and returns "1.0". ok is false for
// anything not window.pane shaped (e.g. "default", "1").
func normalizePaneSpec(s string) (string, bool) {
	s = strings.TrimSpace(s)
	if !paneSpecRe.MatchString(s) {
		return "", false
	}
	return strings.ReplaceAll(s, "_", "."), true
}

// claudePaneSpecFromKey turns a SessionState.AI key like "claude:1_0" into the
// pane spec "1.0". ok is false for non-claude or non-pane keys.
func claudePaneSpecFromKey(aiKey string) (string, bool) {
	const prefix = "claude:"
	if !strings.HasPrefix(aiKey, prefix) {
		return "", false
	}
	return normalizePaneSpec(strings.TrimPrefix(aiKey, prefix))
}

// claudePaneSpecsFromState returns the sorted, de-duplicated pane specs of every
// claude pane recorded in the session's AI map.
func claudePaneSpecsFromState(state *SessionState) []string {
	if state == nil {
		return nil
	}
	var specs []string
	seen := map[string]bool{}
	for key := range state.AI {
		if spec, ok := claudePaneSpecFromKey(key); ok && !seen[spec] {
			specs = append(specs, spec)
			seen[spec] = true
		}
	}
	sort.Strings(specs)
	return specs
}

// looksLikeClaudeCommand reports whether a tmux pane_current_command looks like
// a Claude Code agent. Claude reports its version (e.g. "2.1.186"); shells
// report "zsh"/"bash".
func looksLikeClaudeCommand(cmd string) bool {
	cmd = strings.TrimSpace(cmd)
	if cmd == "" {
		return false
	}
	if strings.EqualFold(cmd, "claude") {
		return true
	}
	return claudeVersionRe.MatchString(cmd)
}

// parsePaneLines parses tab-separated "spec\tcommand\ttitle\tpath" lines from a
// `tmux list-panes -F` invocation.
func parsePaneLines(out string) []tmuxPane {
	var panes []tmuxPane
	for _, line := range strings.Split(strings.TrimRight(out, "\n"), "\n") {
		if strings.TrimSpace(line) == "" {
			continue
		}
		f := strings.SplitN(line, "\t", 4)
		p := tmuxPane{Spec: f[0]}
		if len(f) > 1 {
			p.Command = f[1]
		}
		if len(f) > 2 {
			p.Title = f[2]
		}
		if len(f) > 3 {
			p.Path = f[3]
		}
		panes = append(panes, p)
	}
	return panes
}

// splitTarget splits "session:1.0" into ("session", "1.0"). tmux session names
// can't contain ':', so a LastIndex split is safe even for path-ish names.
func splitTarget(target string) (string, string) {
	idx := strings.LastIndex(target, ":")
	if idx < 0 {
		return target, ""
	}
	return target[:idx], target[idx+1:]
}

func ambiguousNameError(name string, matches []SessionState) error {
	var names []string
	for _, m := range matches {
		entry := m.Name
		if m.Label != "" {
			entry = fmt.Sprintf("%s (%s)", m.Name, m.Label)
		}
		names = append(names, entry)
	}
	sort.Strings(names)
	return fmt.Errorf("%q is ambiguous — matches: %s", name, strings.Join(names, ", "))
}

func multiplePanesError(session string, specs []string) error {
	sorted := append([]string(nil), specs...)
	sort.Strings(sorted)
	return fmt.Errorf("session %q has multiple Claude panes (%s); pick one with --pane", session, strings.Join(sorted, ", "))
}

// resolveCommTargetFromStates resolves a name + optional --pane against a
// snapshot of session states, without touching tmux. needsLive is true when the
// pane couldn't be determined from state alone and the caller must scan live
// panes (no claude AI key, or no state matched the name at all).
func resolveCommTargetFromStates(name, paneFlag string, states []SessionState) (commTarget, bool, error) {
	matches := matchSessionsByName(name, states)
	if len(matches) > 1 {
		return commTarget{}, false, ambiguousNameError(name, matches)
	}
	if len(matches) == 0 {
		// No domux state — caller checks for a live tmux session of this name.
		return commTarget{Name: name, Session: name}, true, nil
	}
	s := matches[0]
	t := commTarget{Name: name, Session: s.Name, Root: s.Root, Label: s.Label}

	if paneFlag != "" {
		spec, ok := normalizePaneSpec(paneFlag)
		if !ok {
			return commTarget{}, false, fmt.Errorf("invalid --pane %q (want window.pane, e.g. 1.0)", paneFlag)
		}
		t.Pane = spec
		t.Target = s.Name + ":" + spec
		return t, false, nil
	}

	specs := claudePaneSpecsFromState(&s)
	switch len(specs) {
	case 1:
		t.Pane = specs[0]
		t.Target = s.Name + ":" + specs[0]
		return t, false, nil
	case 0:
		return t, true, nil
	default:
		return commTarget{}, false, multiplePanesError(s.Name, specs)
	}
}
