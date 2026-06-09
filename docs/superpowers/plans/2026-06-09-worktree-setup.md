# Per-worktree setup (`.domux/worktree.conf`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When domux provisions a worktree (or on demand via `domux setup`), apply a checked-in `.domux/worktree.conf` that `link`s/`copy`s gitignored files (e.g. `CLAUDE.local.md`, `.env`) from the main checkout and `run`s setup commands.

**Architecture:** One new file `worktree_setup.go` holds parsing, the apply engine, two pluggable `run` executors (inline exec for the CLI, tmux send-keys for picker provisioning), and the `domux setup` subcommand. `link`/`copy` are synchronous Go filesystem ops; `run` is dispatched through an injected `setupRunner` so it's testable without a real shell or tmux. `provisionWorkspace` calls the engine after creating the session; a one-line dispatch case in `commands.go` and one help line in `main.go` wire up the CLI.

**Tech Stack:** Go 1.22, stdlib only (`bufio`, `flag`, `os`, `os/exec`, `path/filepath`, `strings`). No new deps. Tests use the existing `setupGitWorkspaceRepo` / `installFakeTmux` helpers in `*_test.go`.

---

## Prerequisites (read before starting)

- **Clean working tree on a feature branch.** The repo currently has uncommitted launch WIP in `commands.go`, `picker.go`, `recap.go`, `session.go`, and tests. Before implementing: commit or stash that WIP, then branch off `main` (or off `launch-prep` if this should ship with the launch). Restricted branches (`main`, `master`, `workspace-*`) must not receive direct commits. This matters because Tasks 5 and 6 edit `commands.go` and `picker.go` — staging must not sweep in unrelated WIP. Use `git add <explicit paths>` (never `git add .`) in every commit step.
- Build/test commands: `go build`, `go test ./...`, single test `go test -run TestName`.

## File Structure

- **Create `worktree_setup.go`** — all setup logic: `setupDirective`, `setupResult`, `setupRunner`, `parseWorktreeConf`, `applyWorktreeSetup`, `runWorktreeSetup`, `linkInto`, `copyInto`, `summarizeSetup`, `inlineRunner`, `sessionRunner`, `tmuxSendKeys`, `resolveMainCheckout`, `gitMainWorktree`, `setupCommand`, const `worktreeConfName`.
- **Create `worktree_setup_test.go`** — unit + end-to-end tests for the above.
- **Modify `commands.go`** — add `case "setup": return setupCommand(args)` to `runCommand` (1 line).
- **Modify `workspaces.go`** — add `SetupSummary` field to `workspaceResult`; call `runWorktreeSetup` at the end of `provisionWorkspace` (~5 lines).
- **Modify `picker.go`** — fold the setup summary into the provision status (1 hunk in `provisionInFocusedGroup`).
- **Modify `main.go`** — add one `domux setup` help line in `printUsage` (1 line).

Existing helpers reused: `fileExists`/`dirExists` (`install.go`), `workspaceRootFromPath` (`workspaces.go`), `os.UserHomeDir` patterns, atomic-write (`tmp`+`os.Rename`) convention.

---

## Task 1: Parse `.domux/worktree.conf`

**Files:**
- Create: `worktree_setup.go`
- Test: `worktree_setup_test.go`

- [ ] **Step 1: Write the failing test**

Create `worktree_setup_test.go`:

```go
package main

import (
	"reflect"
	"strings"
	"testing"
)

func TestParseWorktreeConf(t *testing.T) {
	in := `# bring local files across
link CLAUDE.local.md

copy .env
run npm install
bogus whatever
link    .vscode/settings.json
copy
`
	dirs, warnings := parseWorktreeConf(strings.NewReader(in))

	want := []setupDirective{
		{Verb: "link", Arg: "CLAUDE.local.md"},
		{Verb: "copy", Arg: ".env"},
		{Verb: "run", Arg: "npm install"},
		{Verb: "link", Arg: ".vscode/settings.json"},
	}
	if !reflect.DeepEqual(dirs, want) {
		t.Fatalf("directives = %#v, want %#v", dirs, want)
	}
	// "bogus whatever" (unknown verb) and "copy" (missing arg) → 2 warnings.
	if len(warnings) != 2 {
		t.Fatalf("warnings = %#v, want 2", warnings)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestParseWorktreeConf`
Expected: FAIL — `undefined: parseWorktreeConf` / `undefined: setupDirective`.

- [ ] **Step 3: Write minimal implementation**

Create `worktree_setup.go`:

```go
package main

import (
	"bufio"
	"fmt"
	"io"
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run TestParseWorktreeConf`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add worktree_setup.go worktree_setup_test.go
git commit -m "worktree-setup: parse .domux/worktree.conf"
```

---

## Task 2: Apply engine — `link`, `copy`, dispatch `run`

**Files:**
- Modify: `worktree_setup.go`
- Test: `worktree_setup_test.go`

- [ ] **Step 1: Write the failing test**

Append to `worktree_setup_test.go`:

```go
import (
	"os"
	"path/filepath"
)
// (add the above to the existing import block; "reflect", "strings", "testing"
//  are already imported from Task 1)

func writeFileMode(t *testing.T, path string, data string, mode os.FileMode) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(data), mode); err != nil {
		t.Fatal(err)
	}
}

func TestApplyWorktreeSetupLinkCopyRun(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	writeFileMode(t, filepath.Join(main, "CLAUDE.local.md"), "hi", 0644)
	writeFileMode(t, filepath.Join(main, ".vscode", "settings.json"), "{}", 0644)
	writeFileMode(t, filepath.Join(main, ".env"), "X=1", 0600)
	if err := os.MkdirAll(filepath.Join(main, "assets"), 0755); err != nil {
		t.Fatal(err)
	}

	var ran []string
	run := func(cmd string) error { ran = append(ran, cmd); return nil }

	dirs := []setupDirective{
		{Verb: "link", Arg: "CLAUDE.local.md"},
		{Verb: "link", Arg: ".vscode/settings.json"},
		{Verb: "link", Arg: "assets"},
		{Verb: "copy", Arg: ".env"},
		{Verb: "run", Arg: "echo a"},
		{Verb: "run", Arg: "echo b"},
	}
	results := applyWorktreeSetup(main, wt, dirs, run)

	// link → symlink pointing at the absolute path in main.
	lp := filepath.Join(wt, "CLAUDE.local.md")
	fi, err := os.Lstat(lp)
	if err != nil || fi.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("CLAUDE.local.md not a symlink: %v %v", fi, err)
	}
	if tgt, _ := os.Readlink(lp); tgt != filepath.Join(main, "CLAUDE.local.md") {
		t.Fatalf("symlink target = %q", tgt)
	}
	// nested link: parent dir created in worktree.
	if _, err := os.Lstat(filepath.Join(wt, ".vscode", "settings.json")); err != nil {
		t.Fatalf("nested link missing: %v", err)
	}
	// dir link works.
	if fi, err := os.Lstat(filepath.Join(wt, "assets")); err != nil || fi.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("assets not a symlink: %v %v", fi, err)
	}
	// copy → independent file, mode preserved, NOT a symlink.
	cp := filepath.Join(wt, ".env")
	cfi, err := os.Lstat(cp)
	if err != nil || cfi.Mode()&os.ModeSymlink != 0 {
		t.Fatalf(".env should be a regular file: %v %v", cfi, err)
	}
	if cfi.Mode().Perm() != 0600 {
		t.Fatalf(".env mode = %v, want 0600", cfi.Mode().Perm())
	}
	if b, _ := os.ReadFile(cp); string(b) != "X=1" {
		t.Fatalf(".env content = %q", b)
	}
	// run dispatched in conf order.
	if !reflect.DeepEqual(ran, []string{"echo a", "echo b"}) {
		t.Fatalf("ran = %#v", ran)
	}
	// every directive reported OK.
	for _, r := range results {
		if !r.OK {
			t.Fatalf("directive not OK: %#v", r)
		}
	}
}

func TestApplyWorktreeSetupMissingSourceIsNonFatal(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	run := func(string) error { return nil }
	results := applyWorktreeSetup(main, wt,
		[]setupDirective{{Verb: "link", Arg: "nope.md"}}, run)
	if len(results) != 1 || results[0].OK {
		t.Fatalf("expected one not-OK result, got %#v", results)
	}
	if _, err := os.Lstat(filepath.Join(wt, "nope.md")); !os.IsNotExist(err) {
		t.Fatalf("nothing should have been created, err=%v", err)
	}
}

func TestApplyWorktreeSetupIdempotent(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	writeFileMode(t, filepath.Join(main, "CLAUDE.local.md"), "hi", 0644)
	writeFileMode(t, filepath.Join(main, ".env"), "X=1", 0644)
	dirs := []setupDirective{
		{Verb: "link", Arg: "CLAUDE.local.md"},
		{Verb: "copy", Arg: ".env"},
	}
	run := func(string) error { return nil }
	applyWorktreeSetup(main, wt, dirs, run)
	results := applyWorktreeSetup(main, wt, dirs, run) // second run must not error
	for _, r := range results {
		if !r.OK {
			t.Fatalf("re-run not OK: %#v", r)
		}
	}
	if tgt, _ := os.Readlink(filepath.Join(wt, "CLAUDE.local.md")); tgt != filepath.Join(main, "CLAUDE.local.md") {
		t.Fatalf("symlink wrong after re-run: %q", tgt)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestApplyWorktreeSetup`
Expected: FAIL — `undefined: applyWorktreeSetup` / `undefined: setupResult`.

- [ ] **Step 3: Write minimal implementation**

Add to `worktree_setup.go` (and extend the import block to include `os`, `os/exec`, `path/filepath`):

```go
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
```

Note: `fileExists` already exists in `install.go`. The `var _ = exec.Command` placeholder keeps the `os/exec` import live until Task 4 adds the runners; **delete that line in Task 4**.

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run TestApplyWorktreeSetup`
Expected: PASS (all three apply tests).

- [ ] **Step 5: Commit**

```bash
git add worktree_setup.go worktree_setup_test.go
git commit -m "worktree-setup: link/copy/run apply engine"
```

---

## Task 3: Summary string + `runWorktreeSetup` (read conf, apply)

**Files:**
- Modify: `worktree_setup.go`
- Test: `worktree_setup_test.go`

- [ ] **Step 1: Write the failing test**

Append to `worktree_setup_test.go`:

```go
func TestSummarizeSetup(t *testing.T) {
	results := []setupResult{
		{Verb: "link", OK: true},
		{Verb: "link", OK: true},
		{Verb: "copy", OK: true},
		{Verb: "run", OK: true},
		{Verb: "link", Arg: "x", OK: false, Note: "source missing: x"},
	}
	got := summarizeSetup(results)
	if got != "linked 2, copied 1, ran 1, 1 skipped" {
		t.Fatalf("summary = %q", got)
	}
	if summarizeSetup(nil) != "" {
		t.Fatalf("empty summary should be blank")
	}
}

func TestRunWorktreeSetupNoConf(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	results, err := runWorktreeSetup(main, wt, func(string) error { return nil })
	if err != nil {
		t.Fatalf("err = %v", err)
	}
	if results != nil {
		t.Fatalf("expected nil results when no conf, got %#v", results)
	}
}

func TestRunWorktreeSetupAppliesConf(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	writeFileMode(t, filepath.Join(main, "CLAUDE.local.md"), "hi", 0644)
	writeFileMode(t, filepath.Join(main, ".domux", worktreeConfName),
		"link CLAUDE.local.md\nrun echo hi\n", 0644)
	var ran []string
	results, err := runWorktreeSetup(main, wt, func(c string) error { ran = append(ran, c); return nil })
	if err != nil {
		t.Fatalf("err = %v", err)
	}
	if _, err := os.Lstat(filepath.Join(wt, "CLAUDE.local.md")); err != nil {
		t.Fatalf("link not applied: %v", err)
	}
	if len(ran) != 1 || ran[0] != "echo hi" {
		t.Fatalf("ran = %#v", ran)
	}
	if len(results) != 2 {
		t.Fatalf("results = %#v", results)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestSummarizeSetup|TestRunWorktreeSetup'`
Expected: FAIL — `undefined: summarizeSetup` / `undefined: runWorktreeSetup`.

- [ ] **Step 3: Write minimal implementation**

Add to `worktree_setup.go`:

```go
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run 'TestSummarizeSetup|TestRunWorktreeSetup'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add worktree_setup.go worktree_setup_test.go
git commit -m "worktree-setup: conf reader + summary"
```

---

## Task 4: Run executors — inline (CLI) and tmux send-keys (provision)

**Files:**
- Modify: `worktree_setup.go`
- Test: `worktree_setup_test.go`

- [ ] **Step 1: Write the failing test**

Append to `worktree_setup_test.go`:

```go
func TestInlineRunnerCwdAndEnv(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	run := inlineRunner(main, wt)
	// Command runs via `sh -c` with cwd=wt; write a probe file relatively.
	if err := run(`printf '%s\n%s\n%s\n' "$PWD" "$DOMUX_MAIN" "$DOMUX_WORKTREE" > probe`); err != nil {
		t.Fatalf("run: %v", err)
	}
	b, err := os.ReadFile(filepath.Join(wt, "probe"))
	if err != nil {
		t.Fatalf("probe not written in worktree: %v", err)
	}
	lines := strings.Split(strings.TrimSpace(string(b)), "\n")
	// macOS resolves TempDir through /private; compare via EvalSymlinks.
	wantPWD, _ := filepath.EvalSymlinks(wt)
	gotPWD, _ := filepath.EvalSymlinks(lines[0])
	if gotPWD != wantPWD {
		t.Fatalf("PWD = %q, want %q", gotPWD, wantPWD)
	}
	if lines[1] != main || lines[2] != wt {
		t.Fatalf("env = %#v, want main=%q wt=%q", lines, main, wt)
	}
}

func TestSessionRunnerSendsExportThenCommands(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "calls")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
`, callFile)
	run := sessionRunner("sess", "/main", "/wt")
	if err := run("npm install"); err != nil {
		t.Fatalf("run1: %v", err)
	}
	if err := run("echo done"); err != nil {
		t.Fatalf("run2: %v", err)
	}
	b, _ := os.ReadFile(callFile)
	calls := strings.Split(strings.TrimSpace(string(b)), "\n")
	if len(calls) != 3 {
		t.Fatalf("expected 3 tmux calls (export + 2 cmds), got %#v", calls)
	}
	if !strings.Contains(calls[0], "send-keys") || !strings.Contains(calls[0], "export DOMUX_MAIN") {
		t.Fatalf("first call should export env: %q", calls[0])
	}
	if !strings.Contains(calls[1], "npm install") || !strings.Contains(calls[1], "Enter") {
		t.Fatalf("second call wrong: %q", calls[1])
	}
	if !strings.Contains(calls[2], "echo done") {
		t.Fatalf("third call wrong: %q", calls[2])
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestInlineRunner|TestSessionRunner'`
Expected: FAIL — `undefined: inlineRunner` / `undefined: sessionRunner`.

- [ ] **Step 3: Write minimal implementation**

In `worktree_setup.go`, **delete the `var _ = exec.Command` placeholder line** from Task 2, then add:

```go
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run 'TestInlineRunner|TestSessionRunner'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add worktree_setup.go worktree_setup_test.go
git commit -m "worktree-setup: inline + tmux send-keys run executors"
```

---

## Task 5: `domux setup [--path DIR]` subcommand + main-checkout resolution

**Files:**
- Modify: `worktree_setup.go`
- Modify: `commands.go:82` (add dispatch case)
- Modify: `main.go:27` (help line)
- Test: `worktree_setup_test.go`

- [ ] **Step 1: Write the failing test**

Append to `worktree_setup_test.go`:

```go
func TestResolveMainCheckoutFromWorkspacePath(t *testing.T) {
	// Pure string strip — no git needed for the domux worktree convention.
	wt := "/home/u/proj/.domux/worktrees/workspace-3"
	main, err := resolveMainCheckout(wt)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if main != "/home/u/proj" {
		t.Fatalf("main = %q, want /home/u/proj", main)
	}
}

func TestResolveMainCheckoutFromPlainWorktree(t *testing.T) {
	root := setupGitWorkspaceRepo(t) // root is the main checkout
	wt := filepath.Join(t.TempDir(), "feature")
	gitRun(t, root, "worktree", "add", "-b", "feature", wt)
	main, err := resolveMainCheckout(wt)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	wantMain, _ := filepath.EvalSymlinks(root)
	gotMain, _ := filepath.EvalSymlinks(main)
	if gotMain != wantMain {
		t.Fatalf("main = %q, want %q", gotMain, wantMain)
	}
}

func TestSetupCommandEndToEnd(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	writeFileMode(t, filepath.Join(root, "seed.txt"), "S", 0644)
	writeFileMode(t, filepath.Join(root, ".domux", worktreeConfName),
		"copy seed.txt\nrun true\n", 0644)
	wt := filepath.Join(t.TempDir(), "feature")
	gitRun(t, root, "worktree", "add", "-b", "feature", wt)

	if err := setupCommand([]string{"--path", wt}); err != nil {
		t.Fatalf("setupCommand: %v", err)
	}
	if b, err := os.ReadFile(filepath.Join(wt, "seed.txt")); err != nil || string(b) != "S" {
		t.Fatalf("seed.txt not copied: %q %v", b, err)
	}
}

func TestSetupCommandNoConfIsNoOp(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	wt := filepath.Join(t.TempDir(), "feature")
	gitRun(t, root, "worktree", "add", "-b", "feature", wt)
	if err := setupCommand([]string{"--path", wt}); err != nil {
		t.Fatalf("no-conf setup should not error: %v", err)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestResolveMainCheckout|TestSetupCommand'`
Expected: FAIL — `undefined: resolveMainCheckout` / `undefined: setupCommand`.

- [ ] **Step 3: Write minimal implementation**

Add to `worktree_setup.go` (extend imports with `flag`):

```go
// resolveMainCheckout returns the main checkout for a worktree path. For domux
// workspace dirs it strips the .domux/worktrees/workspace-N suffix; otherwise it
// asks git for the main worktree.
func resolveMainCheckout(worktree string) (string, error) {
	abs, err := filepath.Abs(worktree)
	if err != nil {
		return "", err
	}
	if root, ok := workspaceRootFromPath(abs); ok {
		return root, nil
	}
	return gitMainWorktree(abs)
}

// gitMainWorktree returns the first (main) worktree path from `git worktree list`.
func gitMainWorktree(dir string) (string, error) {
	out, err := exec.Command("git", "-C", dir, "worktree", "list", "--porcelain").Output()
	if err != nil {
		return "", fmt.Errorf("git worktree list: %w", err)
	}
	for _, line := range strings.Split(string(out), "\n") {
		if path, ok := strings.CutPrefix(line, "worktree "); ok {
			return strings.TrimSpace(path), nil
		}
	}
	return "", fmt.Errorf("no worktree found for %s", dir)
}

// setupCommand implements `domux setup [--path DIR]`: applies the main checkout's
// .domux/worktree.conf to DIR (default cwd), running `run` commands inline.
func setupCommand(args []string) error {
	fs := flag.NewFlagSet("setup", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	path := fs.String("path", ".", "worktree directory to set up")
	if err := fs.Parse(args); err != nil {
		return err
	}
	worktree, err := filepath.Abs(*path)
	if err != nil {
		return err
	}
	main, err := resolveMainCheckout(worktree)
	if err != nil {
		return err
	}
	results, err := runWorktreeSetup(main, worktree, inlineRunner(main, worktree))
	if err != nil {
		return err
	}
	if results == nil {
		fmt.Printf("no %s in %s — nothing to set up\n",
			filepath.Join(".domux", worktreeConfName), main)
		return nil
	}
	for _, r := range results {
		if !r.OK {
			fmt.Fprintf(os.Stderr, "  skip %s %s: %s\n", r.Verb, r.Arg, r.Note)
		}
	}
	fmt.Printf("worktree setup: %s\n", summarizeSetup(results))
	return nil
}
```

- [ ] **Step 4: Wire up dispatch in `commands.go`**

In `runCommand`, after the `case "workspace":` block (`commands.go:82-83`), add the `setup` case:

```go
	case "workspace":
		return workspaceCommand(args)
	case "setup":
		return setupCommand(args)
```

- [ ] **Step 5: Add help line in `main.go`**

In `printUsage`, after the `reset-branch` line (`main.go:27`), add:

```go
	fmt.Fprintf(os.Stderr, "  domux reset-branch Reset current git branch only\n")
	fmt.Fprintf(os.Stderr, "  domux setup [DIR]  Apply .domux/worktree.conf to a worktree\n")
```

Note: the flag is `--path`, but the help shows `[DIR]` for brevity; document `--path` in the spec/README later. (Keep the `--path` flag as the canonical interface — it matches the test.)

- [ ] **Step 6: Run tests + build**

Run: `go test -run 'TestResolveMainCheckout|TestSetupCommand' && go build && go vet ./...`
Expected: PASS, clean build, no vet errors.

- [ ] **Step 7: Commit**

```bash
git add worktree_setup.go worktree_setup_test.go commands.go main.go
git commit -m "worktree-setup: domux setup subcommand"
```

---

## Task 6: Hook into `provisionWorkspace` + picker status

**Files:**
- Modify: `workspaces.go:117-185` (struct field + call)
- Modify: `picker.go:1004-1010` (fold summary into status)
- Test: `worktree_setup_test.go`

- [ ] **Step 1: Write the failing test**

Append to `worktree_setup_test.go`:

```go
func TestProvisionWorkspaceAppliesSetup(t *testing.T) {
	root := setupGitWorkspaceRepo(t)
	writeFileMode(t, filepath.Join(root, "CLAUDE.local.md"), "hi", 0644)
	writeFileMode(t, filepath.Join(root, ".domux", worktreeConfName),
		"link CLAUDE.local.md\nrun echo ready\n", 0644)

	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session) exit 1 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	res, err := provisionWorkspace(root)
	if err != nil {
		t.Fatalf("provisionWorkspace: %v", err)
	}
	// link applied in the new worktree.
	if _, err := os.Lstat(filepath.Join(res.Path, "CLAUDE.local.md")); err != nil {
		t.Fatalf("CLAUDE.local.md not linked: %v", err)
	}
	// run command sent into the session via tmux send-keys.
	b, _ := os.ReadFile(callFile)
	if !strings.Contains(string(b), "send-keys") || !strings.Contains(string(b), "echo ready") {
		t.Fatalf("run command not sent to session; tmux calls:\n%s", b)
	}
	// summary populated.
	if res.SetupSummary == "" {
		t.Fatalf("SetupSummary empty")
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestProvisionWorkspaceAppliesSetup`
Expected: FAIL — `res.SetupSummary undefined` (field missing) and link not applied.

- [ ] **Step 3: Add `SetupSummary` to `workspaceResult` and call setup**

In `workspaces.go`, add the field to the struct (`workspaces.go:117-123`):

```go
type workspaceResult struct {
	Path         string
	Branch       string
	Session      string
	BaseBranch   string
	Slot         int
	SetupSummary string
}
```

Then in `provisionWorkspace`, replace the final `return` (currently `workspaces.go:178-184`) so setup runs after the session root is set:

```go
	if _, err := setSessionRoot(session, path); err != nil {
		return workspaceResult{}, err
	}

	// Best-effort: apply .domux/worktree.conf from the main checkout (root).
	// run commands go into the new session so slow setup doesn't block the picker.
	results, _ := runWorktreeSetup(root, path, sessionRunner(session, root, path))

	return workspaceResult{
		Path:         path,
		Branch:       branch,
		Session:      session,
		BaseBranch:   base,
		Slot:         slot,
		SetupSummary: summarizeSetup(results),
	}, nil
```

- [ ] **Step 4: Run the provision test**

Run: `go test -run TestProvisionWorkspaceAppliesSetup`
Expected: PASS.

- [ ] **Step 5: Surface the summary in the picker status**

In `picker.go`, in `provisionInFocusedGroup` (`picker.go:1004-1010`), fold the summary into the status `Value` so the existing `"provisioned %s"` line shows it. Replace:

```go
		res, err := provisionWorkspace(root)
		return pickerActionMsg{
			Action:  "provision",
			Session: res.Session,
			Value:   res.Branch,
			Err:     err,
		}
```

with:

```go
		res, err := provisionWorkspace(root)
		value := res.Branch
		if res.SetupSummary != "" {
			value = fmt.Sprintf("%s (%s)", res.Branch, res.SetupSummary)
		}
		return pickerActionMsg{
			Action:  "provision",
			Session: res.Session,
			Value:   value,
			Err:     err,
		}
```

(`fmt` is already imported in `picker.go`.)

- [ ] **Step 6: Full build + test + vet**

Run: `go build && go test ./... && go vet ./...`
Expected: all PASS, clean build, no vet errors.

- [ ] **Step 7: Commit**

```bash
git add worktree_setup.go worktree_setup_test.go workspaces.go picker.go
git commit -m "worktree-setup: apply on provision + picker status"
```

---

## Task 7: Manual smoke test + README note

**Files:**
- Modify: `README.md` (document the feature)

- [ ] **Step 1: Manual smoke test (real tmux)**

In a real git repo with a tmux server running:

```bash
go build
mkdir -p .domux
printf 'link CLAUDE.local.md\ncopy .env\nrun echo "worktree ready"\n' > .domux/worktree.conf
echo "secret=1" > .env
echo "# local notes" > CLAUDE.local.md
# In the domux switcher, provision a new workspace in this group (the `+` keybind),
# then attach to the new session and confirm:
#   - CLAUDE.local.md is a symlink to the main checkout
#   - .env is an independent copy
#   - "worktree ready" was echoed in the session
ls -l .domux/worktrees/workspace-*/CLAUDE.local.md
# Also test the CLI path against an existing worktree:
./domux setup --path .domux/worktrees/workspace-1
```

Expected: symlink present, `.env` copied, echo ran, `domux setup` prints `worktree setup: linked 1, copied 1, ran 1`.

- [ ] **Step 2: Document in README**

Add a short section to `README.md` describing `.domux/worktree.conf` (the three verbs, that it's checked in, that worktrees should be gitignored via `/.domux/worktrees/`, and the `domux setup [--path DIR]` command). Match the README's existing tone/headings.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "worktree-setup: document .domux/worktree.conf in README"
```

---

## Self-Review

**Spec coverage:**
- Config file location/format/verbs → Tasks 1, 2. ✓
- gitignore guidance → Task 7 (README) + Prerequisites note. ✓
- Auto-apply on provision → Task 6. ✓
- Manual `domux setup [--path]` → Task 5. ✓
- Main-checkout resolution (workspace strip + `git worktree list`) → Task 5 (`resolveMainCheckout`/`gitMainWorktree`). ✓
- link = symlink (files+dirs), copy = file-only, run = command → Task 2. ✓
- run execution: send-keys for picker, inline for CLI; DOMUX_* env + cwd=worktree → Task 4. ✓
- Best-effort/non-fatal → Task 2 (`TestApplyWorktreeSetupMissingSourceIsNonFatal`). ✓
- Testing matrix (parser, link/copy incl. nested+dir, missing source, idempotent, run order, resolution, e2e) → Tasks 1-6. ✓

**Placeholder scan:** No TBD/TODO; every code step shows full code. The one transient placeholder (`var _ = exec.Command` in Task 2) is explicitly removed in Task 4 Step 3. ✓

**Type consistency:** `setupDirective{Verb,Arg}`, `setupResult{Verb,Arg,OK,Note}`, `setupRunner func(string) error`, `applyWorktreeSetup(main, worktree, directives, run)`, `runWorktreeSetup(main, worktree, run) ([]setupResult, error)`, `summarizeSetup([]setupResult) string`, `inlineRunner(main, worktree) setupRunner`, `sessionRunner(session, main, worktree) setupRunner`, `resolveMainCheckout(worktree) (string,error)` — names/signatures consistent across Tasks 1-6 and the `workspaceResult.SetupSummary` field usage. ✓
