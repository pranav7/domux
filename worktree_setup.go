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

// summarizeSetup renders a one-line status like "linked 2, copied 1, ran 1, 1 skipped".
func summarizeSetup(results []setupResult) string {
	var linked, copied, ran, skipped int
	for _, r := range results {
		if !r.OK {
			skipped++
			continue
		}
		switch r.Verb {
		case "link":
			linked++
		case "copy":
			copied++
		case "run":
			ran++
		}
	}
	var parts []string
	if linked > 0 {
		parts = append(parts, fmt.Sprintf("linked %d", linked))
	}
	if copied > 0 {
		parts = append(parts, fmt.Sprintf("copied %d", copied))
	}
	if ran > 0 {
		parts = append(parts, fmt.Sprintf("ran %d", ran))
	}
	if skipped > 0 {
		parts = append(parts, fmt.Sprintf("%d skipped", skipped))
	}
	return strings.Join(parts, ", ")
}

// runWorktreeSetup reads <main>/.domux/worktree.conf and applies it to worktree.
// Returns (nil, nil) when no conf exists — setup is optional. Parse warnings are
// folded into the results as not-OK entries so they show up in the summary.
func runWorktreeSetup(main, worktree string, run setupRunner) ([]setupResult, error) {
	confPath := filepath.Join(main, ".domux", worktreeConfName)
	f, err := os.Open(confPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	defer f.Close()

	directives, warnings := parseWorktreeConf(f)
	results := applyWorktreeSetup(main, worktree, directives, run)
	for _, w := range warnings {
		results = append(results, setupResult{Verb: "conf", OK: false, Note: w})
	}
	return results, nil
}

// inlineRunner runs commands via `sh -c` in the worktree, streaming to stdout/
// stderr, with DOMUX_* env vars set. Used by the `domux setup` CLI.
func inlineRunner(main, worktree string) setupRunner {
	return func(command string) error {
		cmd := exec.Command("sh", "-c", command)
		cmd.Dir = worktree
		cmd.Env = append(os.Environ(),
			"DOMUX_MAIN="+main,
			"DOMUX_WORKTREE="+worktree,
			"DOMUX_ROOT="+main,
		)
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		return cmd.Run()
	}
}

// sessionRunner sends commands into a tmux session via send-keys so long-running
// setup doesn't block the picker and the developer watches it in the new session.
// The session's shell already has cwd = worktree; DOMUX_* vars are exported once,
// lazily, on the first command.
func sessionRunner(session, main, worktree string) setupRunner {
	envSent := false
	return func(command string) error {
		if !envSent {
			export := fmt.Sprintf("export DOMUX_MAIN=%q DOMUX_WORKTREE=%q DOMUX_ROOT=%q",
				main, worktree, main)
			if err := tmuxSendKeys(session, export); err != nil {
				return err
			}
			envSent = true
		}
		return tmuxSendKeys(session, command)
	}
}

func tmuxSendKeys(session, command string) error {
	return exec.Command("tmux", "send-keys", "-t", session, command, "Enter").Run()
}
