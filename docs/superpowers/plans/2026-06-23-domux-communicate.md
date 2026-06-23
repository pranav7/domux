# domux-communicate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a Claude agent in one domux worktree address a peer agent in another worktree *by name* to send it a message and read its output, via three CLI verbs (`send`/`read`/`peek`) plus a `/domux-communicate` plugin skill.

**Architecture:** One new file `communicate.go` holds pure resolution/formatting helpers (the test surface) plus thin `exec.Command("tmux", …)` glue. Subcommands register in `runCommand` (`commands.go`). A standalone plugin at `plugins/domux-communicate/` is registered in `.claude-plugin/marketplace.json`. No new on-disk state — it reads existing `SessionState` files and shells out to tmux.

**Tech Stack:** Go 1.22, `package main` (single-package repo), Go stdlib only (`os/exec`, `regexp`, `text/tabwriter`, `time`), `testing` for tests.

## Global Constraints

- Go 1.22, single root `package main` — no new modules or third-party deps. (verbatim: "Single-package layout (`package main`)… Go 1.22")
- Tmux interop is `exec.Command` only — no tmux library. (verbatim: "Tmux interop is `exec.Command` only.")
- Message text passes as a single argv element to `tmux send-keys -l` — never built into a shell string, so no escaping is needed.
- Atomic writes (`path+".tmp"` → `os.Rename`) for any on-disk state — N/A here (no new state), but don't regress it.
- Test names use `TestBehaviorCondition`. (verbatim AGENTS.md)
- Run `gofmt -w *.go` before commit; `go vet ./...` and `go test ./...` must pass.
- Pane-key convention: `SessionState.AI` keys are `agent:#{window_index}_#{pane_index}` (e.g. `claude:1_0`); the tmux target for that pane is `session:1.0`.
- Do not commit to restricted branches (`main`, `master`, `workspace-*`). Work happens on branch `domux-communicate`.

---

### Task 1: Pure resolution core

**Files:**
- Create: `communicate.go`
- Test: `communicate_test.go`

**Interfaces:**
- Consumes: `SessionState` (fields `Name`, `Root`, `Label`, `AI map[string]string`) from `session.go`.
- Produces:
  - `type commTarget struct { Name, Session, Root, Label, Pane, Target, Command string }`
  - `type tmuxPane struct { Spec, Command, Title, Path string }`
  - `matchSessionsByName(name string, states []SessionState) []SessionState`
  - `normalizePaneSpec(s string) (string, bool)` — `"1_0"`/`"1.0"` → `"1.0"`
  - `claudePaneSpecFromKey(aiKey string) (string, bool)` — `"claude:1_0"` → `"1.0"`
  - `claudePaneSpecsFromState(state *SessionState) []string` — sorted `"w.p"` list
  - `looksLikeClaudeCommand(cmd string) bool`
  - `parsePaneLines(out string) []tmuxPane`
  - `splitTarget(target string) (session, pane string)`
  - `ambiguousNameError(name string, matches []SessionState) error`
  - `multiplePanesError(session string, specs []string) error`
  - `resolveCommTargetFromStates(name, paneFlag string, states []SessionState) (t commTarget, needsLive bool, err error)`

- [ ] **Step 1: Write the failing test**

Create `communicate_test.go`:

```go
package main

import (
	"strings"
	"testing"
)

func st(name, root, label string, ai map[string]string) SessionState {
	return SessionState{Name: name, Root: root, Label: label, AI: ai}
}

func TestMatchSessionsByNameMatchesNameRootBasenameAndLabel(t *testing.T) {
	states := []SessionState{
		st("workspace-2", "/repo/.domux/worktrees/workspace-2", "", nil),
		st("api", "/repo/api", "Billing fix", nil),
		st("docs", "/repo/docs", "", nil),
	}
	cases := []struct {
		name string
		want string // expected single match session name, or "" for none
		n    int    // expected match count
	}{
		{"workspace-2", "workspace-2", 1}, // by session name
		{"WORKSPACE-2", "workspace-2", 1}, // case-insensitive
		{"api", "api", 1},                 // by name
		{"Billing fix", "api", 1},         // by label
		{"docs", "docs", 1},               // by root basename + name
		{"nope", "", 0},
	}
	for _, c := range cases {
		got := matchSessionsByName(c.name, states)
		if len(got) != c.n {
			t.Fatalf("matchSessionsByName(%q) = %d matches, want %d", c.name, len(got), c.n)
		}
		if c.n == 1 && got[0].Name != c.want {
			t.Fatalf("matchSessionsByName(%q) = %q, want %q", c.name, got[0].Name, c.want)
		}
	}
}

func TestMatchSessionsByNameAmbiguousReturnsAll(t *testing.T) {
	states := []SessionState{
		st("alpha", "/repo/alpha", "", nil),
		st("beta", "/repo/beta", "alpha", nil), // label collides with alpha's name
	}
	got := matchSessionsByName("alpha", states)
	if len(got) != 2 {
		t.Fatalf("ambiguous name should match 2 sessions, got %d", len(got))
	}
}

func TestNormalizePaneSpec(t *testing.T) {
	cases := []struct {
		in   string
		want string
		ok   bool
	}{
		{"1_0", "1.0", true},
		{"1.0", "1.0", true},
		{"2_1", "2.1", true},
		{" 1.0 ", "1.0", true},
		{"default", "", false},
		{"", "", false},
		{"abc", "", false},
		{"1", "", false},
	}
	for _, c := range cases {
		got, ok := normalizePaneSpec(c.in)
		if got != c.want || ok != c.ok {
			t.Fatalf("normalizePaneSpec(%q) = (%q,%v), want (%q,%v)", c.in, got, ok, c.want, c.ok)
		}
	}
}

func TestClaudePaneSpecFromKey(t *testing.T) {
	cases := []struct {
		in   string
		want string
		ok   bool
	}{
		{"claude:1_0", "1.0", true},
		{"claude:2_1", "2.1", true},
		{"codex:1_0", "", false},
		{"claude:legacy", "", false},
		{"claude:default", "", false},
		{"claude:", "", false},
		{"garbage", "", false},
	}
	for _, c := range cases {
		got, ok := claudePaneSpecFromKey(c.in)
		if got != c.want || ok != c.ok {
			t.Fatalf("claudePaneSpecFromKey(%q) = (%q,%v), want (%q,%v)", c.in, got, ok, c.want, c.ok)
		}
	}
}

func TestClaudePaneSpecsFromStateSortedAndFiltered(t *testing.T) {
	s := st("ws", "/r", "", map[string]string{
		"claude:2_0": "CLAUDING",
		"claude:1_0": "WAITING",
		"codex:1_0":  "CODEXING",
		"claude:legacy": "CLAUDING",
	})
	got := claudePaneSpecsFromState(&s)
	if strings.Join(got, ",") != "1.0,2.0" {
		t.Fatalf("claudePaneSpecsFromState = %v, want [1.0 2.0]", got)
	}
	if claudePaneSpecsFromState(nil) != nil {
		t.Fatalf("nil state should yield nil specs")
	}
}

func TestLooksLikeClaudeCommand(t *testing.T) {
	cases := map[string]bool{
		"2.1.186": true,
		"2.1":     true,
		"claude":  true,
		"Claude":  true,
		"zsh":     false,
		"bash":    false,
		"nvim":    false,
		"node":    false,
		"":        false,
	}
	for in, want := range cases {
		if got := looksLikeClaudeCommand(in); got != want {
			t.Fatalf("looksLikeClaudeCommand(%q) = %v, want %v", in, got, want)
		}
	}
}

func TestParsePaneLines(t *testing.T) {
	out := "workspace-2:1.0\t2.1.186\tw-2 task\t/repo/ws2\n" +
		"workspace-2:2.0\tzsh\tshell\t/repo/ws2\n\n"
	panes := parsePaneLines(out)
	if len(panes) != 2 {
		t.Fatalf("parsePaneLines = %d panes, want 2", len(panes))
	}
	if panes[0].Spec != "workspace-2:1.0" || panes[0].Command != "2.1.186" || panes[0].Title != "w-2 task" || panes[0].Path != "/repo/ws2" {
		t.Fatalf("first pane parsed wrong: %+v", panes[0])
	}
	if panes[1].Command != "zsh" {
		t.Fatalf("second pane command = %q, want zsh", panes[1].Command)
	}
}

func TestSplitTarget(t *testing.T) {
	s, p := splitTarget("workspace-2:1.0")
	if s != "workspace-2" || p != "1.0" {
		t.Fatalf("splitTarget = (%q,%q), want (workspace-2,1.0)", s, p)
	}
	s, p = splitTarget("noColon")
	if s != "noColon" || p != "" {
		t.Fatalf("splitTarget(noColon) = (%q,%q)", s, p)
	}
}

func TestResolveCommTargetFromStates(t *testing.T) {
	states := []SessionState{
		st("workspace-2", "/r/workspace-2", "", map[string]string{"claude:1_0": "WAITING"}),
		st("multi", "/r/multi", "", map[string]string{"claude:1_0": "CLAUDING", "claude:2_0": "WAITING"}),
		st("shellonly", "/r/shellonly", "", nil),
		st("dup", "/r/dup", "", nil),
		st("dup2", "/r/other", "dup", nil),
	}

	// 1. Single claude pane in AI map → resolved, no live needed.
	tgt, live, err := resolveCommTargetFromStates("workspace-2", "", states)
	if err != nil || live {
		t.Fatalf("workspace-2: err=%v live=%v", err, live)
	}
	if tgt.Target != "workspace-2:1.0" {
		t.Fatalf("workspace-2 target = %q, want workspace-2:1.0", tgt.Target)
	}

	// 2. Explicit --pane wins.
	tgt, live, err = resolveCommTargetFromStates("multi", "2.0", states)
	if err != nil || live || tgt.Target != "multi:2.0" {
		t.Fatalf("multi --pane 2.0 → target=%q live=%v err=%v", tgt.Target, live, err)
	}

	// 3. Multiple claude panes, no --pane → error.
	if _, _, err := resolveCommTargetFromStates("multi", "", states); err == nil {
		t.Fatalf("multi without --pane should be an error")
	}

	// 4. No claude pane in state → needsLive.
	_, live, err = resolveCommTargetFromStates("shellonly", "", states)
	if err != nil || !live {
		t.Fatalf("shellonly should need live lookup: live=%v err=%v", live, err)
	}

	// 5. No state match → needsLive with Session=name.
	tgt, live, err = resolveCommTargetFromStates("ghost", "", states)
	if err != nil || !live || tgt.Session != "ghost" {
		t.Fatalf("ghost → live=%v err=%v session=%q", live, err, tgt.Session)
	}

	// 6. Ambiguous name → error.
	if _, _, err := resolveCommTargetFromStates("dup", "", states); err == nil {
		t.Fatalf("ambiguous 'dup' should error")
	}

	// 7. Bad --pane → error.
	if _, _, err := resolveCommTargetFromStates("workspace-2", "garbage", states); err == nil {
		t.Fatalf("bad --pane should error")
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestMatchSessionsByName|TestNormalizePaneSpec|TestClaudePaneSpec|TestLooksLikeClaude|TestParsePaneLines|TestSplitTarget|TestResolveCommTargetFromStates' ./...`
Expected: FAIL — `undefined: matchSessionsByName` (and the other symbols).

- [ ] **Step 3: Write minimal implementation**

Create `communicate.go`:

```go
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
	claudeVersionRe = regexp.MustCompile(`^\d+\.\d+`)
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `gofmt -w communicate.go communicate_test.go && go test -run 'TestMatchSessionsByName|TestNormalizePaneSpec|TestClaudePaneSpec|TestLooksLikeClaude|TestParsePaneLines|TestSplitTarget|TestResolveCommTargetFromStates' ./...`
Expected: PASS (ok).

- [ ] **Step 5: Commit**

```bash
git add communicate.go communicate_test.go
git commit -m "communicate: pure name/pane resolution helpers"
```

---

### Task 2: Attribution and message formatting

**Files:**
- Modify: `communicate.go`
- Test: `communicate_test.go`

**Interfaces:**
- Produces:
  - `attributionPrefix(from string) string`
  - `formatPeerMessage(from, message string) string`

- [ ] **Step 1: Write the failing test**

Append to `communicate_test.go`:

```go
func TestAttributionPrefixNamed(t *testing.T) {
	got := attributionPrefix("workspace-3")
	if !strings.Contains(got, "workspace-3") {
		t.Fatalf("prefix should name the sender: %q", got)
	}
	if !strings.Contains(strings.ToLower(got), "not your operator") {
		t.Fatalf("prefix should disclaim the human operator: %q", got)
	}
}

func TestAttributionPrefixEmptyFrom(t *testing.T) {
	got := attributionPrefix("  ")
	if strings.Contains(got, "\"\"") {
		t.Fatalf("empty from should not produce empty quotes: %q", got)
	}
	if !strings.Contains(strings.ToLower(got), "peer claude agent") {
		t.Fatalf("empty from should still identify a peer agent: %q", got)
	}
}

func TestFormatPeerMessageKeepsBodyVerbatim(t *testing.T) {
	body := "Pull branch `feat/x`; see $HOME/doc.md \"now\""
	got := formatPeerMessage("ws-3", body)
	if !strings.HasSuffix(got, "\n\n"+body) {
		t.Fatalf("body must be appended verbatim after a blank line: %q", got)
	}
	if !strings.HasPrefix(got, "[domux peer message") {
		t.Fatalf("message must start with the attribution prefix: %q", got)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestAttributionPrefix|TestFormatPeerMessage' ./...`
Expected: FAIL — `undefined: attributionPrefix`.

- [ ] **Step 3: Write minimal implementation**

Append to `communicate.go` (no new imports — `fmt`/`strings` already imported):

```go
// attributionPrefix builds the header that tells the receiving agent the
// message came from a peer agent, not the human operator.
func attributionPrefix(from string) string {
	from = strings.TrimSpace(from)
	if from == "" {
		return "[domux peer message — from a peer Claude agent, not your operator]"
	}
	return fmt.Sprintf("[domux peer message — from worktree %q, a peer Claude agent (not your operator)]", from)
}

// formatPeerMessage prepends the attribution header to the caller's message.
func formatPeerMessage(from, message string) string {
	return attributionPrefix(from) + "\n\n" + message
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `gofmt -w communicate.go communicate_test.go && go test -run 'TestAttributionPrefix|TestFormatPeerMessage' ./...`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add communicate.go communicate_test.go
git commit -m "communicate: peer-attribution message formatting"
```

---

### Task 3: tmux interop, live resolution, and idle detection

**Files:**
- Modify: `communicate.go`

**Interfaces:**
- Consumes: `tmuxSessionExists` (commands.go), `execOutput` (state_commands.go), `loadSessionStateWithLegacy`/`listSessionStates`/`normalizeAIState` (session.go), `currentTmuxSession`/`currentTmuxPaneKey`.
- Produces:
  - `listSessionPanes(session string) ([]tmuxPane, error)`
  - `listAllPanes() ([]tmuxPane, error)`
  - `paneCurrentCommand(target string) string`
  - `resolveLivePane(t commTarget) (commTarget, error)`
  - `resolveCommTarget(name, paneFlag string) (commTarget, error)`
  - `capturePaneTail(target string, lines int) string`
  - `paneAIState(session, pane string) string`
  - `paneBusyByCapture(target string) bool`
  - `isPaneBusy(t commTarget) bool`
  - `tmuxSendLiteral(target, text string) error`
  - `tmuxSendEnter(target string) error`
  - `isOwnPane(target string) bool`
  - `currentSenderName() string`

This task is tmux-shelling glue, integration-level by nature. Like the existing tmux helpers (`createTmuxSession`, `attachTmuxSession`) it isn't unit-tested; it's verified by `go build`/`go vet` here and exercised in the demo (Task 6). Its pure inputs (`parsePaneLines`, `resolveCommTargetFromStates`, `looksLikeClaudeCommand`) are already covered by Task 1.

- [ ] **Step 1: Add imports**

Edit the import block in `communicate.go` to add `os/exec` and `time`:

```go
import (
	"fmt"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
)
```

- [ ] **Step 2: Write the implementation**

Append to `communicate.go`:

```go
// listSessionPanes lists the panes of one session (Spec is "window.pane").
func listSessionPanes(session string) ([]tmuxPane, error) {
	out, err := exec.Command("tmux", "list-panes", "-t", session, "-F",
		"#{window_index}.#{pane_index}\t#{pane_current_command}\t#{pane_title}\t#{pane_current_path}").CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("tmux list-panes -t %s: %w: %s", session, err, strings.TrimSpace(string(out)))
	}
	return parsePaneLines(string(out)), nil
}

// listAllPanes lists panes across every session (Spec is "session:window.pane").
func listAllPanes() ([]tmuxPane, error) {
	out, err := exec.Command("tmux", "list-panes", "-a", "-F",
		"#{session_name}:#{window_index}.#{pane_index}\t#{pane_current_command}\t#{pane_title}\t#{pane_current_path}").CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("tmux list-panes -a: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return parsePaneLines(string(out)), nil
}

func paneCurrentCommand(target string) string {
	if target == "" {
		return ""
	}
	out, err := execOutput("tmux", "display-message", "-t", target, "-p", "#{pane_current_command}")
	if err != nil {
		return ""
	}
	return out
}

// resolveLivePane finds the Claude pane in t.Session by scanning live panes,
// used when state alone couldn't pin it down.
func resolveLivePane(t commTarget) (commTarget, error) {
	if !tmuxSessionExists(t.Session) {
		return commTarget{}, fmt.Errorf("no worktree or tmux session matches %q (try: domux peek)", t.Name)
	}
	panes, err := listSessionPanes(t.Session)
	if err != nil {
		return commTarget{}, err
	}
	var claude []tmuxPane
	for _, p := range panes {
		if looksLikeClaudeCommand(p.Command) {
			claude = append(claude, p)
		}
	}
	switch {
	case len(claude) == 1:
		t.Pane, t.Command = claude[0].Spec, claude[0].Command
	case len(claude) == 0 && len(panes) == 1:
		t.Pane, t.Command = panes[0].Spec, panes[0].Command
	case len(claude) == 0:
		return commTarget{}, fmt.Errorf("no Claude agent pane found in session %q (use --pane window.pane)", t.Session)
	default:
		specs := make([]string, 0, len(claude))
		for _, p := range claude {
			specs = append(specs, p.Spec)
		}
		return commTarget{}, multiplePanesError(t.Session, specs)
	}
	t.Target = t.Session + ":" + t.Pane
	return t, nil
}

// resolveCommTarget resolves a worktree name (and optional --pane) to a peer
// agent's tmux pane: session state first, live tmux fallback second.
func resolveCommTarget(name, paneFlag string) (commTarget, error) {
	states, err := listSessionStates()
	if err != nil {
		return commTarget{}, err
	}
	t, needsLive, err := resolveCommTargetFromStates(name, paneFlag, states)
	if err != nil {
		return commTarget{}, err
	}
	if !needsLive {
		t.Command = paneCurrentCommand(t.Target)
		return t, nil
	}
	return resolveLivePane(t)
}

// capturePaneTail returns the trimmed last `lines` of a pane (for busy diffing).
func capturePaneTail(target string, lines int) string {
	out, err := execOutput("tmux", "capture-pane", "-t", target, "-p", "-S", fmt.Sprintf("-%d", lines))
	if err != nil {
		return ""
	}
	return out
}

// paneAIState returns the normalized claude AI state for a target pane from
// session state, or "" if unknown/idle.
func paneAIState(session, pane string) string {
	state := loadSessionStateWithLegacy(session)
	key := "claude:" + strings.ReplaceAll(pane, ".", "_")
	return normalizeAIState("claude", state.AI[key])
}

// paneBusyByCapture captures the pane tail twice with a short gap; changed
// output means the pane is actively producing.
func paneBusyByCapture(target string) bool {
	first := capturePaneTail(target, 20)
	time.Sleep(600 * time.Millisecond)
	return first != capturePaneTail(target, 20)
}

// isPaneBusy reports whether the peer agent at target is actively generating.
// domux AI state is the cheap reliable signal when present; otherwise fall back
// to a capture-first diff.
func isPaneBusy(t commTarget) bool {
	switch paneAIState(t.Session, t.Pane) {
	case "CLAUDING", "COMPACTING":
		return true
	case "WAITING":
		return false
	}
	return paneBusyByCapture(t.Target)
}

func tmuxSendLiteral(target, text string) error {
	out, err := exec.Command("tmux", "send-keys", "-t", target, "-l", text).CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux send-keys -l: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func tmuxSendEnter(target string) error {
	out, err := exec.Command("tmux", "send-keys", "-t", target, "Enter").CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux send-keys Enter: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

// isOwnPane reports whether target is the caller's own pane (guards self-sends).
func isOwnPane(target string) bool {
	session, err := currentTmuxSession()
	if err != nil {
		return false
	}
	own := session + ":" + strings.ReplaceAll(currentTmuxPaneKey(), "_", ".")
	return target == own
}

// currentSenderName is the default --from: the caller's domux label, else its
// tmux session name, else "".
func currentSenderName() string {
	session, err := currentTmuxSession()
	if err != nil {
		return ""
	}
	if state := loadSessionStateWithLegacy(session); state.Label != "" {
		return state.Label
	}
	return session
}
```

- [ ] **Step 3: Verify it builds and vets**

Run: `gofmt -w communicate.go && go build ./... && go vet ./...`
Expected: no output (success). The existing test suite still passes: `go test ./...` → ok.

- [ ] **Step 4: Commit**

```bash
git add communicate.go
git commit -m "communicate: tmux interop, live pane resolution, idle detection"
```

---

### Task 4: send / read / peek commands, dispatch, and usage

**Files:**
- Modify: `communicate.go`
- Modify: `commands.go:17-105` (add cases to `runCommand`)
- Modify: `main.go:36` (add an "Agents:" block to `printUsage`)
- Test: `communicate_test.go`

**Interfaces:**
- Consumes: everything from Tasks 1-3.
- Produces:
  - `sendCommand(args []string) error`
  - `readCommand(args []string) error`
  - `peekCommand(args []string) error`
  - `type peekRow struct { Name, Label, Target, State, Task string }`
  - `peekRows(panes []tmuxPane, states []SessionState) []peekRow`
  - `peekStateLabel(state *SessionState, paneSpec string) string`

- [ ] **Step 1: Write the failing test**

Append to `communicate_test.go`:

```go
func TestPeekStateLabel(t *testing.T) {
	s := st("ws", "/r", "", map[string]string{
		"claude:1_0": "CLAUDING",
		"claude:2_0": "WAITING",
		"claude:3_0": "IDLE",
	})
	cases := map[string]string{
		"1.0": "working",
		"2.0": "waiting",
		"3.0": "idle", // IDLE normalizes to "" → idle
		"9.0": "idle", // absent → idle
	}
	for pane, want := range cases {
		if got := peekStateLabel(&s, pane); got != want {
			t.Fatalf("peekStateLabel(%q) = %q, want %q", pane, got, want)
		}
	}
}

func TestPeekRowsFiltersToClaudePanesAndJoinsState(t *testing.T) {
	panes := []tmuxPane{
		{Spec: "workspace-2:1.0", Command: "2.1.186", Title: "w-2 task"},
		{Spec: "workspace-2:2.0", Command: "zsh", Title: "shell"},
		{Spec: "ghost:1.0", Command: "2.1.0", Title: "no state here"},
	}
	states := []SessionState{
		st("workspace-2", "/r/workspace-2", "Billing", map[string]string{"claude:1_0": "CLAUDING"}),
	}
	rows := peekRows(panes, states)
	if len(rows) != 2 {
		t.Fatalf("peekRows = %d rows, want 2 (claude panes only)", len(rows))
	}
	// rows sorted by Target: "ghost:1.0" then "workspace-2:1.0"
	if rows[0].Target != "ghost:1.0" || rows[0].State != "unknown" {
		t.Fatalf("ghost row wrong: %+v", rows[0])
	}
	if rows[1].Target != "workspace-2:1.0" || rows[1].State != "working" || rows[1].Label != "Billing" || rows[1].Name != "workspace-2" {
		t.Fatalf("workspace-2 row wrong: %+v", rows[1])
	}
}

func TestSendCommandRejectsMissingMessage(t *testing.T) {
	if err := sendCommand([]string{"workspace-2"}); err == nil {
		t.Fatalf("send with no message should error")
	}
	if err := readCommand([]string{}); err == nil {
		t.Fatalf("read with no name should error")
	}
	if err := peekCommand([]string{"extra"}); err == nil {
		t.Fatalf("peek with args should error")
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestPeekStateLabel|TestPeekRows|TestSendCommandRejectsMissingMessage' ./...`
Expected: FAIL — `undefined: peekStateLabel` (and others).

- [ ] **Step 3: Add imports and implement the commands**

Edit the import block in `communicate.go` to add `flag`, `os`, and `text/tabwriter`:

```go
import (
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"text/tabwriter"
	"time"
)
```

Append to `communicate.go`:

```go
// sendCommand: domux send [flags] <worktree> <message…>
// Flags must precede the positionals (Go flag parsing stops at the first
// non-flag arg).
func sendCommand(args []string) error {
	fs := flag.NewFlagSet("send", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	from := fs.String("from", "", "attribution name (defaults to the current session)")
	pane := fs.String("pane", "", "target pane as window.pane, e.g. 1.0")
	noEnter := fs.Bool("no-enter", false, "stage the text in the peer's input box without submitting")
	wait := fs.Bool("wait", false, "wait until the peer is idle before sending")
	waitTimeout := fs.Duration("wait-timeout", 60*time.Second, "max time to wait with --wait")
	if err := fs.Parse(args); err != nil {
		return err
	}
	rest := fs.Args()
	if len(rest) < 2 {
		return fmt.Errorf("usage: domux send [--from NAME] [--pane W.P] [--no-enter] [--wait] <worktree> <message…>")
	}
	name := rest[0]
	message := strings.Join(rest[1:], " ")

	t, err := resolveCommTarget(name, *pane)
	if err != nil {
		return err
	}
	if isOwnPane(t.Target) {
		return fmt.Errorf("refusing to send to your own pane (%s)", t.Target)
	}

	attribution := *from
	if attribution == "" {
		attribution = currentSenderName()
	}

	queued := false
	if *wait {
		deadline := time.Now().Add(*waitTimeout)
		for isPaneBusy(t) {
			if time.Now().After(deadline) {
				return fmt.Errorf("peer %s still busy after %s — not sent (retry, or drop --wait to queue it)", t.Target, *waitTimeout)
			}
			time.Sleep(750 * time.Millisecond)
		}
	} else {
		queued = isPaneBusy(t)
	}

	if err := tmuxSendLiteral(t.Target, formatPeerMessage(attribution, message)); err != nil {
		return err
	}
	if !*noEnter {
		if err := tmuxSendEnter(t.Target); err != nil {
			return err
		}
	}

	switch {
	case *noEnter:
		fmt.Printf("staged in %s (%s) — not submitted (--no-enter)\n", t.Name, t.Target)
	case queued:
		fmt.Printf("sent to %s (%s) — peer was generating, message queued\n", t.Name, t.Target)
	default:
		fmt.Printf("sent to %s (%s)\n", t.Name, t.Target)
	}
	return nil
}

// readCommand: domux read [flags] <worktree>
func readCommand(args []string) error {
	fs := flag.NewFlagSet("read", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	lines := fs.Int("lines", 50, "number of trailing lines to capture")
	pane := fs.String("pane", "", "target pane as window.pane, e.g. 1.0")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("usage: domux read [--lines N] [--pane W.P] <worktree>")
	}
	if *lines < 1 {
		return fmt.Errorf("--lines must be >= 1")
	}
	t, err := resolveCommTarget(fs.Arg(0), *pane)
	if err != nil {
		return err
	}
	out, err := exec.Command("tmux", "capture-pane", "-t", t.Target, "-p", "-S", fmt.Sprintf("-%d", *lines)).CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux capture-pane -t %s: %w: %s", t.Target, err, strings.TrimSpace(string(out)))
	}
	os.Stdout.Write(out)
	return nil
}

// peekCommand: domux peek — list running Claude agents across worktrees.
func peekCommand(args []string) error {
	if len(args) != 0 {
		return fmt.Errorf("peek does not accept arguments")
	}
	panes, err := listAllPanes()
	if err != nil {
		return err
	}
	states, err := listSessionStates()
	if err != nil {
		return err
	}
	rows := peekRows(panes, states)
	if len(rows) == 0 {
		fmt.Println("no running Claude agents found")
		return nil
	}
	w := tabwriter.NewWriter(os.Stdout, 0, 2, 2, ' ', 0)
	fmt.Fprintln(w, "NAME\tSTATE\tTASK\tTARGET")
	for _, r := range rows {
		name := r.Name
		if r.Label != "" {
			name = fmt.Sprintf("%s (%s)", r.Name, r.Label)
		}
		fmt.Fprintf(w, "%s\t%s\t%s\t%s\n", name, r.State, truncateTask(r.Task), r.Target)
	}
	return w.Flush()
}

// peekRow is one line of `domux peek` output.
type peekRow struct {
	Name   string // addressable session name
	Label  string // human label, if any
	Target string // session:pane
	State  string // working / waiting / compacting / idle / unknown
	Task   string // pane title (the agent's current task)
}

// peekRows turns live panes + session states into sorted peek rows, keeping only
// panes that look like Claude agents.
func peekRows(panes []tmuxPane, states []SessionState) []peekRow {
	byName := map[string]SessionState{}
	for _, s := range states {
		byName[s.Name] = s
	}
	var rows []peekRow
	for _, p := range panes {
		if !looksLikeClaudeCommand(p.Command) {
			continue
		}
		session, paneSpec := splitTarget(p.Spec)
		row := peekRow{Name: session, Target: p.Spec, Task: p.Title, State: "unknown"}
		if s, ok := byName[session]; ok {
			row.Label = s.Label
			row.State = peekStateLabel(&s, paneSpec)
		}
		rows = append(rows, row)
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].Target < rows[j].Target })
	return rows
}

// peekStateLabel maps a pane's claude AI state to a peek-friendly word.
func peekStateLabel(state *SessionState, paneSpec string) string {
	key := "claude:" + strings.ReplaceAll(paneSpec, ".", "_")
	switch normalizeAIState("claude", state.AI[key]) {
	case "CLAUDING":
		return "working"
	case "WAITING":
		return "waiting"
	case "COMPACTING":
		return "compacting"
	default:
		return "idle"
	}
}

func truncateTask(s string) string {
	s = strings.TrimSpace(s)
	if len(s) > 50 {
		return s[:47] + "..."
	}
	return s
}
```

- [ ] **Step 4: Register the subcommands**

In `commands.go`, inside the `switch name` in `runCommand` (after the `case "caffeinate":` block, before `default:` at line ~95-103), add:

```go
	case "send":
		return sendCommand(args)
	case "read":
		return readCommand(args)
	case "peek":
		return peekCommand(args)
```

- [ ] **Step 5: Add usage lines**

In `main.go`, in `printUsage`, after the "Setup:" block (after the `domux doctor` line at line 35, before the "Status output" block at line 36), add:

```go
	fmt.Fprintf(os.Stderr, "Agents:\n")
	fmt.Fprintf(os.Stderr, "  domux peek            List running Claude agents across worktrees\n")
	fmt.Fprintf(os.Stderr, "  domux send NAME MSG   Send a message to another worktree's agent\n")
	fmt.Fprintf(os.Stderr, "  domux read NAME       Read another worktree agent's recent output\n\n")
```

- [ ] **Step 6: Run tests and build**

Run: `gofmt -w communicate.go communicate_test.go commands.go main.go && go test ./... && go vet ./...`
Expected: `ok` for the package, no vet output. The new tests (`TestPeekStateLabel`, `TestPeekRows…`, `TestSendCommandRejectsMissingMessage`) pass.

- [ ] **Step 7: Commit**

```bash
git add communicate.go communicate_test.go commands.go main.go
git commit -m "communicate: add send/read/peek subcommands + usage"
```

---

### Task 5: domux-communicate plugin

**Files:**
- Create: `plugins/domux-communicate/.claude-plugin/plugin.json`
- Create: `plugins/domux-communicate/skills/domux-communicate/SKILL.md`
- Modify: `.claude-plugin/marketplace.json`
- Modify: `manifest_test.go`

**Interfaces:**
- Produces: a registered plugin `domux-communicate` exposing `/domux-communicate`.

- [ ] **Step 1: Write the failing test**

In `manifest_test.go`, extend `TestMarketplaceManifestIsValid` to assert the new plugin is listed. After the `if len(m.Plugins) == 0 {…}` block (line ~30-32), add:

```go
	wantPlugins := map[string]bool{"implement-pipeline": false, "domux-communicate": false}
	for _, p := range m.Plugins {
		if _, ok := wantPlugins[p.Name]; ok {
			wantPlugins[p.Name] = true
		}
	}
	for name, found := range wantPlugins {
		if !found {
			t.Fatalf("marketplace.json missing plugin %q", name)
		}
	}
```

And add a new test for the plugin manifest + skill frontmatter:

```go
func TestCommunicatePluginManifestAndSkill(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("plugins", "domux-communicate", ".claude-plugin", "plugin.json"))
	if err != nil {
		t.Fatalf("read plugin.json: %v", err)
	}
	var p struct {
		Name        string `json:"name"`
		Version     string `json:"version"`
		Description string `json:"description"`
	}
	if err := json.Unmarshal(data, &p); err != nil {
		t.Fatalf("parse plugin.json: %v", err)
	}
	if p.Name != "domux-communicate" {
		t.Fatalf("plugin name = %q, want \"domux-communicate\"", p.Name)
	}
	if p.Version == "" || p.Description == "" {
		t.Fatalf("plugin version/description must be set: %+v", p)
	}

	skill := filepath.Join("plugins", "domux-communicate", "skills", "domux-communicate", "SKILL.md")
	sdata, err := os.ReadFile(skill)
	if err != nil {
		t.Fatalf("read SKILL.md: %v", err)
	}
	text := string(sdata)
	if len(text) < 10 || text[:4] != "---\n" {
		t.Fatalf("SKILL.md missing frontmatter")
	}
	head := text
	if len(head) > 500 {
		head = head[:500]
	}
	if !contains(head, "name:") || !contains(head, "description:") {
		t.Fatalf("SKILL.md frontmatter must declare name and description")
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run 'TestMarketplaceManifestIsValid|TestCommunicatePluginManifestAndSkill' ./...`
Expected: FAIL — `marketplace.json missing plugin "domux-communicate"` / `read plugin.json: no such file`.

- [ ] **Step 3: Create the plugin manifest**

Create `plugins/domux-communicate/.claude-plugin/plugin.json`:

```json
{
  "name": "domux-communicate",
  "version": "0.1.0",
  "description": "Talk to the Claude agent in another domux worktree: address a peer by name to send it a message and read its output (domux send/read/peek).",
  "author": { "name": "pranav7" },
  "homepage": "https://github.com/pranav7/domux"
}
```

- [ ] **Step 4: Create the skill**

Create `plugins/domux-communicate/skills/domux-communicate/SKILL.md`:

```markdown
---
name: domux-communicate
description: Use when you (an agent in one domux worktree) need to hand work to, message, or read the output of the Claude agent running in ANOTHER worktree. Covers the canonical peer handoff — "I built X on branch Y, please pull it in" — plus discovering which agents are running. Wraps `domux peek` / `domux send` / `domux read`.
---

# /domux-communicate — message a peer agent in another worktree

domux runs each worktree's Claude agent in its own tmux pane. These three
commands let you address a **peer agent by worktree name** — you never scan
panes yourself; domux resolves the name to the right pane from the session
mapping it already maintains.

The canonical use is a **handoff**: you finished something on your branch and
the agent in another worktree needs it ("I built X on branch Y, here's the doc,
please pull it in").

> The peer is another Claude agent, not the human operator. Every message you
> send is automatically prefixed to say so, so the peer doesn't mistake it for
> its user.

## 1. Discover who's running — `domux peek`

```
domux peek
```

Lists every running Claude agent across worktrees: its **NAME** (what you pass
to `send`/`read`), **STATE** (working / waiting / idle), **TASK**, and tmux
**TARGET**. Run this first if you're unsure of the exact name.

## 2. Send a message — `domux send <name> <message…>`

```
domux send workspace-2 "I pushed the fix on branch eng-225-agent-calc. Pull it: git fetch && git checkout eng-225-agent-calc. The doc is docs/eng-225.md."
```

- **Name** is the worktree's session name, its directory/branch basename, or its
  domux label — any of them resolve.
- **Flags come before the positionals** (Go flag parsing stops at the first
  non-flag word):
  - `--from NAME` — how to attribute the message. Defaults to your own session's
    label or name, so the peer sees who it's from.
  - `--pane W.P` — pick a specific pane (e.g. `--pane 2.0`) when a session runs
    more than one agent.
  - `--no-enter` — type the message into the peer's input box but **don't**
    submit it, so a human (or you, later) can review before sending.
  - `--wait` (with optional `--wait-timeout 2m`) — block until the peer is idle,
    then send. Without it, the message sends immediately; if the peer is
    mid-generation Claude Code **queues** it (nothing is lost) and `send` tells
    you it was queued.

The whole message is sent literally via tmux, so backticks, quotes, `$`, paths,
and semicolons are safe — no escaping needed. Long messages show up in the
peer's input as a "Pasted text" block but still submit in full.

`send` refuses to message your own pane.

## 3. Read the reply — `domux read <name> [--lines N]`

```
domux read workspace-2 --lines 80
```

Prints the peer pane's recent output (default 50 lines). Use it to confirm the
peer picked up your message and see what it did.

## Typical handoff flow

```
domux peek                                   # find the peer, check it's idle/waiting
domux send workspace-2 "<the handoff>"       # hand it off (attributed to you)
domux read workspace-2 --lines 80            # later: see that it acted on it
```

## When it can't resolve a name

- *"no worktree or tmux session matches X"* → run `domux peek` for valid names.
- *"X is ambiguous"* → two sessions share that name/label; use the exact session
  name from `domux peek`.
- *"session X has multiple Claude panes"* → add `--pane W.P` (see the target
  column in `domux peek`).
```

- [ ] **Step 5: Register in the marketplace**

Edit `.claude-plugin/marketplace.json` — add a second entry to the `plugins` array (after the `implement-pipeline` object):

```json
    {
      "name": "domux-communicate",
      "source": "./plugins/domux-communicate",
      "description": "Message the Claude agent in another domux worktree: domux send/read/peek + the /domux-communicate skill."
    }
```

(Remember the comma after the `implement-pipeline` object.)

- [ ] **Step 6: Run tests to verify they pass**

Run: `go test -run 'TestMarketplaceManifestIsValid|TestCommunicatePluginManifestAndSkill|TestPluginManifestIsValid' ./...`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add plugins/domux-communicate .claude-plugin/marketplace.json manifest_test.go
git commit -m "communicate: add /domux-communicate plugin skill"
```

---

### Task 6: README docs and full verification

**Files:**
- Modify: `README.md`

**Interfaces:** none (docs + verification only).

- [ ] **Step 1: Find the insertion point**

Run: `grep -n '^## ' README.md`
Pick the section that lists commands/usage (e.g. after the main commands/usage section, before any "Install"/"Development" section). Insert the new section there so it reads naturally alongside the other command docs.

- [ ] **Step 2: Add the README section**

Insert this section at the chosen point in `README.md`:

```markdown
## Talk to agents in other worktrees

When you run several worktrees side by side (see `domux start` / workspaces),
the Claude agent in one worktree can message the agent in another — addressing
it **by name**, never by hunting for tmux panes. This powers handoffs like
"I built X on branch Y, please pull it in."

```sh
domux peek                              # list running agents: name, state, task, target
domux send workspace-2 "Pushed the fix on branch eng-225; please rebase onto it."
domux read workspace-2 --lines 80       # read the peer's recent output
```

- **Name** resolves against a session's name, its worktree dir/branch basename,
  or its domux label.
- `domux send` prefixes every message so the peer knows it came from a **peer
  agent, not the human** (`--from NAME` overrides the attribution). The text is
  sent literally through tmux, so quotes, backticks, and paths need no escaping.
- By default a message sent to a busy peer is **queued** by Claude Code; pass
  `--wait` to block until the peer is idle first, or `--no-enter` to stage the
  text without submitting it.

The same workflow is available to agents as the `/domux-communicate` plugin
skill (`plugins/domux-communicate`).
```

- [ ] **Step 3: Full verification**

Run:
```bash
gofmt -l *.go            # expect: no output (all formatted)
go build ./...           # expect: success
go vet ./...             # expect: no output
go test ./...            # expect: ok
```
All four must be clean.

- [ ] **Step 4: Demo capture for the PR (real tmux)**

Build and exercise the commands against the live tmux server to capture demo output for the PR description:
```bash
go build -o domux .
./domux peek
# Pick an idle peer NAME from the output that is NOT this session, then:
# ./domux send <peer> --no-enter "domux-communicate smoke test (staged, not submitted)"
# ./domux read <peer> --lines 20
```
Record the `peek` table and a `send`/`read` round-trip for the PR. Use
`--no-enter` for the smoke test so you don't actually interrupt a peer; do a
real submit only against a scratch/idle session.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "communicate: document send/read/peek + plugin in README"
```

---

## Self-Review

**Spec coverage:**
- Resolve by name via existing mapping → Task 1 (`matchSessionsByName`, `resolveCommTargetFromStates`) + Task 3 (`resolveCommTarget` live fallback). ✓
- `domux send` (types then submits; `--no-enter`, `--from`) → Task 4. ✓
- `domux read [--lines N]` → Task 4. ✓
- `domux peek` (list running agents) → Task 4. ✓
- Plugin skill `/domux-communicate` shipped in plugins dir → Task 5. ✓
- Safe quoting (backticks/paths/quotes) → `tmuxSendLiteral` uses `exec.Command` argv, no shell (Task 3); asserted by `TestFormatPeerMessageKeepsBodyVerbatim` (Task 2). ✓
- Capture-first idle check → Task 3 (`isPaneBusy`/`paneBusyByCapture`), wired into `send` default-queue and `--wait` (Task 4). ✓
- Attribution prefix (peer, not human) → Task 2; default `--from` in Task 3/4. ✓
- Tests in existing style → Tasks 1, 2, 4, 5. ✓
- README/docs update → Task 6. ✓
- Demo in PR → Task 6 Step 4. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code; commands and expected output are concrete. ✓

**Type consistency:** `commTarget`/`tmuxPane` defined in Task 1 and used unchanged in Tasks 3-4. `resolveCommTargetFromStates` (Task 1) returns `(commTarget, bool, error)`, consumed by `resolveCommTarget` (Task 3). `peekRow`/`peekRows`/`peekStateLabel` defined and used in Task 4. `normalizeAIState("claude", …)`, `loadSessionStateWithLegacy`, `currentTmuxPaneKey`, `execOutput`, `tmuxSessionExists`, `listSessionStates` all match existing signatures in `session.go`/`state_commands.go`/`commands.go`. ✓
```
