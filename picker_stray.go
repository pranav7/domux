package main

import (
	"os"
	"os/exec"
	"strings"
)

// Agents shell out to `tmux new-session -d` to park a command somewhere, then
// leave the session behind. Those leftovers pile up in the switcher as rows
// that carry nothing — an idle shell in a worktree, named after whatever the
// agent was doing that hour ("fixture-stack", "pr1-stack"). This file decides
// which live sessions are that kind of debris so the picker can drop them.
//
// The test errs toward showing: every signal must agree before a session is
// hidden, and a probe that fails to read yields "not stray". A stray row is a
// small annoyance; a session that vanishes because tmux hiccuped is a bug you
// cannot see.

// straySessionProbe holds the four cheap facts about one live tmux session that
// decide whether it is an agent leftover.
type straySessionProbe struct {
	// HasDomuxState is true when domux has a state file for the session, i.e.
	// it was created by `domux start`/a workspace, or explicitly adopted,
	// labelled, pinned, or otherwise touched. Legacy ~/.tmux-* dotfiles are not
	// consulted: domux always writes them alongside the state file, so the file
	// is the complete record.
	HasDomuxState bool
	Attached      bool
	// AllAutoNamed is true when every window in the session is still on tmux
	// automatic-rename — nobody has named one by hand.
	AllAutoNamed bool
	// PaneCmds is pane_current_command for every pane in the session.
	PaneCmds []string
}

// isStray reports whether the switcher should hide this session.
func (p straySessionProbe) isStray() bool {
	if p.HasDomuxState || p.Attached || !p.AllAutoNamed {
		return false
	}
	if len(p.PaneCmds) == 0 {
		return false
	}
	for _, cmd := range p.PaneCmds {
		if !isIdleShellCommand(cmd) {
			return false
		}
	}
	return true
}

// idleShellCommands are the pane_current_command values that mean "a prompt is
// sitting there and nothing is running". Anything else — nvim, node, a version
// string like "2.1.247" for the Claude CLI — counts as work in progress.
var idleShellCommands = map[string]bool{
	"sh": true, "bash": true, "zsh": true, "fish": true,
	"dash": true, "ksh": true, "mksh": true, "csh": true, "tcsh": true,
	"login": true,
}

func isIdleShellCommand(cmd string) bool {
	return idleShellCommands[strings.TrimSpace(cmd)]
}

type liveSession struct {
	Name     string
	Attached bool
}

// parseSessionListLines reads `tmux list-sessions -F
// '#{session_name}\t#{session_attached}'` output.
func parseSessionListLines(out string) []liveSession {
	var sessions []liveSession
	for _, line := range strings.Split(out, "\n") {
		name, attached, _ := strings.Cut(strings.TrimRight(line, "\r"), "\t")
		if name == "" {
			continue
		}
		sessions = append(sessions, liveSession{Name: name, Attached: attached == "1"})
	}
	return sessions
}

// parseAutoNamedSessions reads `tmux list-windows -a -F
// '#{session_name}\t#{?automatic-rename,1,0}'` output into session name → every
// window still auto-named. One manually named window turns the session false.
func parseAutoNamedSessions(out string) map[string]bool {
	autoNamed := map[string]bool{}
	for _, line := range strings.Split(out, "\n") {
		session, auto, ok := strings.Cut(strings.TrimRight(line, "\r"), "\t")
		if session == "" || !ok {
			continue
		}
		isAuto := auto == "1"
		if seen, ok := autoNamed[session]; ok {
			isAuto = isAuto && seen
		}
		autoNamed[session] = isAuto
	}
	return autoNamed
}

// parsePaneCommandsBySession reads `tmux list-panes -a -F
// '#{session_name}\t#{pane_current_command}'` output into session name → the
// command running in each of its panes.
func parsePaneCommandsBySession(out string) map[string][]string {
	bySession := map[string][]string{}
	for _, line := range strings.Split(out, "\n") {
		session, cmd, ok := strings.Cut(strings.TrimRight(line, "\r"), "\t")
		if session == "" || !ok || cmd == "" {
			continue
		}
		bySession[session] = append(bySession[session], cmd)
	}
	return bySession
}

// straySessionFilter answers isStray for every live session from two
// whole-server tmux probes. Both are best-effort: an unreadable probe leaves its
// map empty, which makes every session non-stray.
type straySessionFilter struct {
	autoNamed map[string]bool
	paneCmds  map[string][]string
}

func newStraySessionFilter() straySessionFilter {
	windows, err := exec.Command("tmux", "list-windows", "-a", "-F",
		"#{session_name}\t#{?automatic-rename,1,0}").Output()
	if err != nil {
		debugLog("stray filter: list-windows -a failed: %v", err)
	}
	panes, err := exec.Command("tmux", "list-panes", "-a", "-F",
		"#{session_name}\t#{pane_current_command}").Output()
	if err != nil {
		debugLog("stray filter: list-panes -a failed: %v", err)
	}
	return straySessionFilter{
		autoNamed: parseAutoNamedSessions(string(windows)),
		paneCmds:  parsePaneCommandsBySession(string(panes)),
	}
}

// isStray reports whether sess is an agent leftover the switcher should hide.
func (f straySessionFilter) isStray(sess liveSession) bool {
	probe := straySessionProbe{
		HasDomuxState: domuxTracksSession(sess.Name),
		Attached:      sess.Attached,
		AllAutoNamed:  f.autoNamed[sess.Name],
		PaneCmds:      f.paneCmds[sess.Name],
	}
	if !probe.isStray() {
		return false
	}
	debugLog("stray filter: hiding %q (no domux state, detached, auto-named, panes %v)",
		sess.Name, probe.PaneCmds)
	return true
}

// domuxTracksSession reports whether domux has a state file for this session.
func domuxTracksSession(session string) bool {
	path, err := sessionStatePath(session)
	if err != nil {
		return false
	}
	_, err = os.Stat(path)
	return err == nil
}
