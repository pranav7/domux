package main

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const worktreeConfName = "worktree.conf"

// setupDirective is one parsed line of .domux/worktree.conf.
type setupDirective struct {
	Verb string // link | copy | run
	Arg  string // path (link/copy) or command (run); rest-of-line, trimmed
}

// parseWorktreeConf reads line-oriented directives. Blank lines and #-comments
// are ignored. Unknown verbs and empty args become warnings (skipped), so an
// older domux binary degrades gracefully against a newer conf.
func parseWorktreeConf(r io.Reader) (directives []setupDirective, warnings []string) {
	sc := bufio.NewScanner(r)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		verb, rest, _ := strings.Cut(line, " ")
		arg := strings.TrimSpace(rest)
		switch verb {
		case "link", "copy", "run":
			if arg == "" {
				warnings = append(warnings, fmt.Sprintf("%s: missing argument", verb))
				continue
			}
			directives = append(directives, setupDirective{Verb: verb, Arg: arg})
		default:
			warnings = append(warnings, fmt.Sprintf("unknown directive %q", line))
		}
	}
	return directives, warnings
}

// setupResult records the outcome of one directive for summaries/warnings.
type setupResult struct {
	Verb string
	Arg  string
	OK   bool
	Note string // failure reason or short detail
}

// setupRunner executes a `run` command. Injected so callers choose inline exec
// (CLI) or tmux send-keys (provisioning), and tests can capture commands.
type setupRunner func(command string) error

// applyWorktreeSetup performs link/copy synchronously and dispatches run through
// the runner, in conf order. Best-effort: a failed directive is recorded but
// never aborts the rest.
func applyWorktreeSetup(main, worktree string, directives []setupDirective, run setupRunner) []setupResult {
	results := make([]setupResult, 0, len(directives))
	for _, d := range directives {
		res := setupResult{Verb: d.Verb, Arg: d.Arg, OK: true}
		var err error
		switch d.Verb {
		case "link":
			err = linkInto(main, worktree, d.Arg)
		case "copy":
			err = copyInto(main, worktree, d.Arg)
		case "run":
			err = run(d.Arg)
		}
		if err != nil {
			res.OK = false
			res.Note = err.Error()
		}
		results = append(results, res)
	}
	return results
}

// linkInto symlinks worktree/<rel> → absolute <main>/<rel>. Works for files and
// directories. Creates parent dirs in the worktree; removes a pre-existing entry
// first so re-runs are idempotent.
func linkInto(main, worktree, rel string) error {
	src := filepath.Join(main, rel)
	if !fileExists(src) {
		return fmt.Errorf("source missing: %s", rel)
	}
	dst := filepath.Join(worktree, rel)
	if err := os.MkdirAll(filepath.Dir(dst), 0755); err != nil {
		return err
	}
	if _, err := os.Lstat(dst); err == nil {
		if err := os.RemoveAll(dst); err != nil {
			return err
		}
	}
	return os.Symlink(src, dst)
}

// copyInto copies main/<rel> → worktree/<rel> as an independent file, preserving
// mode. Atomic via tmp+rename (domux convention). File-only.
func copyInto(main, worktree, rel string) error {
	src := filepath.Join(main, rel)
	info, err := os.Stat(src)
	if err != nil {
		return fmt.Errorf("source missing: %s", rel)
	}
	if info.IsDir() {
		return fmt.Errorf("copy is file-only; use link for dir %s", rel)
	}
	data, err := os.ReadFile(src)
	if err != nil {
		return err
	}
	dst := filepath.Join(worktree, rel)
	if err := os.MkdirAll(filepath.Dir(dst), 0755); err != nil {
		return err
	}
	tmp := dst + ".tmp"
	if err := os.WriteFile(tmp, data, info.Mode().Perm()); err != nil {
		return err
	}
	return os.Rename(tmp, dst)
}

var _ = exec.Command // exec used by runners added in Task 4
