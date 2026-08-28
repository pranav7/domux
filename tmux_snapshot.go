package main

import (
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
)

// The switcher's row model needs, for every live session: its current pane's
// cwd, its windows (index, name, active, active-pane cwd), every pane's tty, and
// — for the stray filter — every pane's command and whether all windows are
// still auto-named. tmux exposes window and session variables inside a pane
// format, so one whole-server `list-panes -a` answers all of it at once.
//
// This matters because process spawns, not tmux itself, dominate the switcher's
// startup: each fork+exec costs milliseconds while the query it runs costs
// microseconds. Asking per session (`display-message` + `list-windows` +
// `list-panes`, three spawns each) put startup a few hundred milliseconds behind
// its first paint; asking once puts it at one spawn regardless of session count.
// Add fields to tmuxServerPaneFormat rather than adding a second probe.

const tmuxServerPaneFormat = "#{session_name}\t#{session_attached}\t" +
	"#{window_index}\t#{window_name}\t#{window_active}\t#{?automatic-rename,1,0}\t" +
	"#{pane_active}\t#{pane_current_command}\t#{pane_tty}\t#{pane_current_path}"

const tmuxServerPaneFields = 10

// serverPane is one pane on the tmux server, with the window and session facts
// tmux reports alongside it.
type serverPane struct {
	Session      string
	Attached     bool
	WindowIndex  int
	WindowName   string
	WindowActive bool
	AutoNamed    bool
	PaneActive   bool
	Command      string
	TTY          string
	Path         string
}

// parseTmuxServerPanes reads `tmux list-panes -a -F tmuxServerPaneFormat`
// output. Lines with too few fields or a non-integer window index are skipped:
// a pane we cannot read is better dropped than guessed at.
func parseTmuxServerPanes(out string) []serverPane {
	var panes []serverPane
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		f := strings.SplitN(line, "\t", tmuxServerPaneFields)
		if len(f) < tmuxServerPaneFields {
			continue
		}
		idx, err := strconv.Atoi(f[2])
		if err != nil {
			continue
		}
		panes = append(panes, serverPane{
			Session:      f[0],
			Attached:     f[1] == "1",
			WindowIndex:  idx,
			WindowName:   f[3],
			WindowActive: f[4] == "1",
			AutoNamed:    f[5] == "1",
			PaneActive:   f[6] == "1",
			Command:      f[7],
			TTY:          f[8],
			Path:         f[9],
		})
	}
	return panes
}

// tmuxServerSnapshot is one whole-server pane list, indexed by session. Every
// accessor is a map lookup over already-fetched data, so callers may ask freely.
// A session absent from the snapshot yields zero values, matching the old
// behaviour when a per-session tmux call failed.
type tmuxServerSnapshot struct {
	bySession map[string][]serverPane
}

func newTmuxServerSnapshot(panes []serverPane) tmuxServerSnapshot {
	bySession := make(map[string][]serverPane)
	for _, p := range panes {
		bySession[p.Session] = append(bySession[p.Session], p)
	}
	return tmuxServerSnapshot{bySession: bySession}
}

// readTmuxServerSnapshot runs the single probe. A failure yields an empty
// snapshot rather than an error: the switcher still renders every session it
// knows from state, just without live window detail.
func readTmuxServerSnapshot() tmuxServerSnapshot {
	out, err := exec.Command("tmux", "list-panes", "-a", "-F", tmuxServerPaneFormat).Output()
	if err != nil {
		debugLog("snapshot: list-panes -a failed: %v", err)
		return newTmuxServerSnapshot(nil)
	}
	return newTmuxServerSnapshot(parseTmuxServerPanes(string(out)))
}

// sessionPath is the session's current directory: the cwd of the active pane in
// its active window. This is what `display-message -t S -p
// '#{pane_current_path}'` reports.
func (s tmuxServerSnapshot) sessionPath(session string) string {
	for _, p := range s.bySession[session] {
		if p.WindowActive && p.PaneActive {
			return p.Path
		}
	}
	return ""
}

// windows returns one row per window, in server order, carrying the active
// pane's cwd. Windows are discovered from their panes, so a window always has
// at least one.
func (s tmuxServerSnapshot) windows(session string) []windowInfo {
	var windows []windowInfo
	at := map[int]int{} // window index → position in windows
	for _, p := range s.bySession[session] {
		i, seen := at[p.WindowIndex]
		if !seen {
			windows = append(windows, windowInfo{
				Index:  p.WindowIndex,
				Name:   p.WindowName,
				Active: p.WindowActive,
			})
			i = len(windows) - 1
			at[p.WindowIndex] = i
		}
		// windowInfo.Path is the active pane's cwd; fall back to the first pane
		// so a window whose active pane we somehow missed still has a path.
		if p.PaneActive || windows[i].Path == "" {
			windows[i].Path = p.Path
		}
	}
	return windows
}

// paneTTYs maps each window index to the ttys of all its panes. A window may
// hold several panes (domux's Claude pane beside a working pane), and a recap
// matches a claude session against any of them.
func (s tmuxServerSnapshot) paneTTYs(session string) map[int][]string {
	ttys := map[int][]string{}
	for _, p := range s.bySession[session] {
		if p.TTY == "" {
			continue
		}
		ttys[p.WindowIndex] = append(ttys[p.WindowIndex], p.TTY)
	}
	return ttys
}

// allAutoNamed reports whether every window in the session is still on tmux
// automatic-rename. A session we know nothing about is not auto-named, so an
// empty snapshot never hides anything (see straySessionProbe).
func (s tmuxServerSnapshot) allAutoNamed(session string) bool {
	panes := s.bySession[session]
	if len(panes) == 0 {
		return false
	}
	for _, p := range panes {
		if !p.AutoNamed {
			return false
		}
	}
	return true
}

// paneCommands is pane_current_command for every pane in the session.
func (s tmuxServerSnapshot) paneCommands(session string) []string {
	var cmds []string
	for _, p := range s.bySession[session] {
		if p.Command != "" {
			cmds = append(cmds, p.Command)
		}
	}
	return cmds
}

// gitFacts is what the switcher needs to know about a worktree: the repo root it
// belongs to and the branch checked out there. Either may be empty.
type gitFacts struct {
	Root   string
	Branch string
}

// gitRootBranchArgs asks for both facts in one process. Two `git -C p` calls per
// session was the other half of the switcher's startup cost.
func gitRootBranchArgs(path string) []string {
	return []string{"-C", path, "rev-parse", "--show-toplevel", "--abbrev-ref", "HEAD"}
}

// parseGitRootBranch reads `git rev-parse --show-toplevel --abbrev-ref HEAD`
// stdout: the toplevel, then the branch. Both are optional, because rev-parse
// writes what it can before failing — an unborn HEAD (fresh `git init`) prints
// the toplevel and then exits non-zero, and we keep the root.
//
// A detached HEAD makes --abbrev-ref echo "HEAD", which is not a branch name;
// the `git branch --show-current` this replaced returned "" there, so we do too.
func parseGitRootBranch(out string) gitFacts {
	lines := strings.Split(strings.TrimSpace(out), "\n")
	// The read is positional, so it is only trustworthy once line 0 validates as
	// a toplevel. git prints the toplevel first or not at all — so a first line
	// that is not an absolute path means this is output we do not understand,
	// and line 1 is not a branch either. Report nothing rather than guess.
	if len(lines) == 0 || !filepath.IsAbs(strings.TrimSpace(lines[0])) {
		return gitFacts{}
	}
	facts := gitFacts{Root: strings.TrimSpace(lines[0])}
	if len(lines) > 1 {
		if branch := strings.TrimSpace(lines[1]); branch != "HEAD" {
			facts.Branch = branch
		}
	}
	return facts
}

// gitFactsForPaths probes every distinct non-empty path concurrently, keyed by
// the path asked for. Sessions sharing a worktree cost one call, and the wall
// time is one git call rather than one per session.
func gitFactsForPaths(paths []string) map[string]gitFacts {
	unique := make(map[string]bool, len(paths))
	for _, p := range paths {
		if p != "" {
			unique[p] = true
		}
	}
	if len(unique) == 0 {
		return nil
	}

	facts := make(map[string]gitFacts, len(unique))
	var mu sync.Mutex
	var wg sync.WaitGroup
	for path := range unique {
		wg.Add(1)
		go func(path string) {
			defer wg.Done()
			// stdout is read even on a non-zero exit: rev-parse writes the
			// toplevel before it fails on an unborn HEAD.
			out, _ := exec.Command("git", gitRootBranchArgs(path)...).Output()
			f := parseGitRootBranch(string(out))
			mu.Lock()
			facts[path] = f
			mu.Unlock()
		}(path)
	}
	wg.Wait()
	return facts
}
