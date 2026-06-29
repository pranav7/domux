# Resume Saved Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `domux resume` to recreate tmux work sessions from domux's already-persisted session state after a restart, landing the user in the switcher with progress shown inline.

**Architecture:** A new `resume.go` splits a pure, unit-testable planner (`planResume` partitions saved states into recreate-vs-prune) from a side-effecting executor (`executeResumeStep` recreates one tmux session or prunes one dead state file). The picker gains an optional `*resumeJob` that drives sequential execution via background `tea.Cmd`s and narrates progress in its existing status box; the existing 2-second refresh loop fills in the live session list as sessions come back.

**Tech Stack:** Go 1.22, bubbletea/lipgloss TUI, `exec.Command` for git/tmux, standard library only.

## Global Constraints

- Go 1.22, single `package main`, no module subdirs. Copy these verbatim into every task's mental model.
- Atomic on-disk writes via tmp + `os.Rename`. Resume only writes state through existing helpers (`saveSessionState`, `setSessionRoot`, `removeSessionState`) that already do this — never hand-roll a write.
- tmux and git are invoked via `exec.Command` only — no library.
- Reuse existing helpers; do NOT redefine them: `dirExists` / `fileExists` (`install.go`), `workspaceRootFromPath` (`workspaces.go`), `createTmuxSession` / `tmuxSessionExists` / `clearSessionStateFiles` (`commands.go`), `setSessionRoot` / `listSessionStates` / `removeSessionState` / `sessionStatePath` / `loadSessionState` (`session.go`).
- New subcommands register in `runCommand` (`commands.go`) and parse with `flag.NewFlagSet(name, flag.ContinueOnError)` + `fs.SetOutput(os.Stderr)`.
- Test helpers already exist and MUST be reused: `installFakeTmux(t, script, callFile)` (`commands_test.go:389`), `setupGitWorkspaceRepo(t)` / `gitRun` (`workspaces_test.go`). Tmux is shimmed via a fake on `PATH` that logs calls to `$DOMUX_TMUX_CALL`. Per-test isolation uses `t.Setenv("HOME", t.TempDir())`.

---

### Task 1: Pure resume planner

**Files:**
- Create: `resume.go`
- Test: `resume_test.go`

**Interfaces:**
- Consumes: `SessionState` (`session.go`), `workspaceRootFromPath(path string) (string, bool)` (`workspaces.go`), `dirExists(path string) bool` (`install.go`).
- Produces:
  - `type resumeStatus string` with consts `resumeRecreated`, `resumeRunning`, `resumePruned` (values `"recreated"`, `"already running"`, `"pruned"`).
  - `type resumeTarget struct { Name, Root, Group string; Prune bool; Status resumeStatus; Err error }`.
  - `func resumeGroup(root string) string`
  - `func planResume(states []SessionState, filter string) (recreate, prune []resumeTarget)`
  - `func sortResumeTargets(ts []resumeTarget)`
  - `func availableResumeGroups(states []SessionState) []string`

- [ ] **Step 1: Write the failing tests**

Create `resume_test.go`:

```go
package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResumeGroupStripsWorkspaceSuffix(t *testing.T) {
	got := resumeGroup("/home/me/proj/.domux/worktrees/workspace-2")
	if got != "proj" {
		t.Fatalf("resumeGroup = %q, want proj", got)
	}
	if got := resumeGroup("/home/me/proj"); got != "proj" {
		t.Fatalf("resumeGroup(main) = %q, want proj", got)
	}
}

func TestPlanResumePartitionsMissingRoots(t *testing.T) {
	live := t.TempDir()
	states := []SessionState{
		{Name: "alive", Root: live},
		{Name: "dead", Root: filepath.Join(t.TempDir(), "gone")},
	}

	recreate, prune := planResume(states, "")
	if len(recreate) != 1 || recreate[0].Name != "alive" {
		t.Fatalf("recreate = %#v, want [alive]", recreate)
	}
	if len(prune) != 1 || prune[0].Name != "dead" || !prune[0].Prune {
		t.Fatalf("prune = %#v, want [dead] with Prune=true", prune)
	}
}

func TestPlanResumeWorkspaceSharesMainGroup(t *testing.T) {
	base := t.TempDir()
	main := filepath.Join(base, "proj")
	ws := filepath.Join(main, ".domux", "worktrees", "workspace-1")
	if err := os.MkdirAll(ws, 0755); err != nil {
		t.Fatal(err)
	}
	states := []SessionState{
		{Name: "proj", Root: main},
		{Name: "workspace-1", Root: ws},
	}

	recreate, prune := planResume(states, "proj")
	if len(prune) != 0 {
		t.Fatalf("prune = %#v, want empty", prune)
	}
	if len(recreate) != 2 {
		t.Fatalf("recreate = %#v, want both states (shared group)", recreate)
	}
}

func TestPlanResumeFilterCaseInsensitive(t *testing.T) {
	base := t.TempDir()
	main := filepath.Join(base, "proj")
	if err := os.MkdirAll(main, 0755); err != nil {
		t.Fatal(err)
	}
	states := []SessionState{{Name: "proj", Root: main}}

	recreate, _ := planResume(states, "PROJ")
	if len(recreate) != 1 {
		t.Fatalf("recreate = %#v, want 1 (case-insensitive match)", recreate)
	}
}

func TestPlanResumeSkipsBlankNameOrRoot(t *testing.T) {
	states := []SessionState{
		{Name: "", Root: t.TempDir()},
		{Name: "noroot", Root: ""},
	}
	recreate, prune := planResume(states, "")
	if len(recreate) != 0 || len(prune) != 0 {
		t.Fatalf("recreate=%#v prune=%#v, want both empty", recreate, prune)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test -run 'TestResumeGroup|TestPlanResume' ./...`
Expected: FAIL — `undefined: resumeGroup`, `undefined: planResume`.

- [ ] **Step 3: Write minimal implementation**

Create `resume.go`:

```go
package main

import (
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test -run 'TestResumeGroup|TestPlanResume' ./...`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add resume.go resume_test.go
git commit -m "feat: add pure resume planner"
```

---

### Task 2: Resume step executor

**Files:**
- Modify: `resume.go` (add `executeResumeStep`)
- Test: `resume_test.go` (add cases)

**Interfaces:**
- Consumes: `tmuxSessionExists(name string) bool`, `createTmuxSession(name, root string) error` (`commands.go`), `setSessionRoot(session, root string) (*SessionState, error)`, `removeSessionState(session string) error` (`session.go`), `clearSessionStateFiles(homeDir, session string) error` (`commands.go`), `resumeTarget` / status consts (Task 1).
- Produces: `func executeResumeStep(t resumeTarget) resumeTarget` — performs one target's side effect and returns it with `Status`/`Err` filled.

- [ ] **Step 1: Write the failing tests**

Add to `resume_test.go`:

```go
import "strings" // add alongside existing imports

func TestExecuteResumeStepRecreatesMissingSession(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session) exit 1 ;;
new-session) exit 0 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())
	root := t.TempDir()

	got := executeResumeStep(resumeTarget{Name: "sess", Root: root})
	if got.Err != nil {
		t.Fatalf("unexpected err: %v", got.Err)
	}
	if got.Status != resumeRecreated {
		t.Fatalf("Status = %q, want recreated", got.Status)
	}
	data, _ := os.ReadFile(callFile)
	if !strings.Contains(string(data), "new-session") {
		t.Fatalf("tmux new-session not invoked; calls=%q", data)
	}
	st, err := loadSessionState("sess")
	if err != nil || st.Root != root {
		t.Fatalf("session state Root = %q (err %v), want %q", st.Root, err, root)
	}
}

func TestExecuteResumeStepSkipsExistingSession(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
has-session) exit 0 ;;
*) exit 0 ;;
esac
`, callFile)
	t.Setenv("HOME", t.TempDir())

	got := executeResumeStep(resumeTarget{Name: "sess", Root: t.TempDir()})
	if got.Status != resumeRunning {
		t.Fatalf("Status = %q, want already running", got.Status)
	}
	data, _ := os.ReadFile(callFile)
	if strings.Contains(string(data), "new-session") {
		t.Fatalf("should not create existing session; calls=%q", data)
	}
}

func TestExecuteResumeStepPruneRemovesState(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := saveSessionState(&SessionState{Name: "dead", Root: "/nonexistent"}); err != nil {
		t.Fatal(err)
	}
	legacy := filepath.Join(home, ".tmux-label-dead")
	if err := os.WriteFile(legacy, []byte("x"), 0644); err != nil {
		t.Fatal(err)
	}

	got := executeResumeStep(resumeTarget{Name: "dead", Prune: true})
	if got.Status != resumePruned || got.Err != nil {
		t.Fatalf("Status=%q Err=%v, want pruned/nil", got.Status, got.Err)
	}
	p, _ := sessionStatePath("dead")
	if fileExists(p) {
		t.Fatalf("state file not removed: %s", p)
	}
	if fileExists(legacy) {
		t.Fatalf("legacy file not removed: %s", legacy)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test -run TestExecuteResumeStep ./...`
Expected: FAIL — `undefined: executeResumeStep`.

- [ ] **Step 3: Write minimal implementation**

Add to `resume.go` (add `"os"` to the import block):

```go
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test -run TestExecuteResumeStep ./...`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add resume.go resume_test.go
git commit -m "feat: add resume step executor"
```

---

### Task 3: Resume job state machine

**Files:**
- Modify: `resume.go` (add `resumeJob`, methods, `resumeStepMsg`, `resumeStepCmd`)
- Test: `resume_test.go` (add cases)

**Interfaces:**
- Consumes: `resumeTarget` / status consts (Task 1), `executeResumeStep` (Task 2), `tea.Cmd` / `tea.Msg` (bubbletea).
- Produces:
  - `type resumeJob struct { queue []resumeTarget; pos int; done bool; nRecreated, nRunning, nPruned, nFailed int }`
  - `func (j *resumeJob) nextTarget() (resumeTarget, bool)`
  - `func (j *resumeJob) record(t resumeTarget)`
  - `func (j *resumeJob) summary() string`
  - `type resumeStepMsg struct { target resumeTarget }`
  - `func resumeStepCmd(t resumeTarget) tea.Cmd`

- [ ] **Step 1: Write the failing tests**

Add to `resume_test.go`:

```go
func TestResumeJobRecordAndNext(t *testing.T) {
	j := &resumeJob{queue: []resumeTarget{{Name: "a"}, {Name: "b"}}}

	first, ok := j.nextTarget()
	if !ok || first.Name != "a" {
		t.Fatalf("nextTarget = %q ok=%v, want a", first.Name, ok)
	}
	j.record(resumeTarget{Name: "a", Status: resumeRecreated})
	if j.pos != 1 || j.nRecreated != 1 {
		t.Fatalf("pos=%d nRecreated=%d, want 1/1", j.pos, j.nRecreated)
	}
	second, ok := j.nextTarget()
	if !ok || second.Name != "b" {
		t.Fatalf("nextTarget = %q ok=%v, want b", second.Name, ok)
	}
	j.record(resumeTarget{Name: "b", Status: resumePruned})
	if _, ok := j.nextTarget(); ok {
		t.Fatalf("nextTarget should be exhausted")
	}
}

func TestResumeJobRecordCountsByStatus(t *testing.T) {
	j := &resumeJob{}
	j.record(resumeTarget{Status: resumeRecreated})
	j.record(resumeTarget{Status: resumeRunning})
	j.record(resumeTarget{Status: resumePruned})
	j.record(resumeTarget{Err: os.ErrPermission})
	if j.nRecreated != 1 || j.nRunning != 1 || j.nPruned != 1 || j.nFailed != 1 {
		t.Fatalf("counts = %#v, want 1 each", j)
	}
}

func TestResumeJobSummary(t *testing.T) {
	j := &resumeJob{nRecreated: 6, nPruned: 1}
	if got := j.summary(); got != "restored 6 · running 0 · pruned 1" {
		t.Fatalf("summary = %q", got)
	}
	j.nFailed = 2
	if got := j.summary(); !strings.Contains(got, "2 failed") {
		t.Fatalf("summary = %q, want failed count", got)
	}
}

func TestResumeStepCmdPrunes(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := saveSessionState(&SessionState{Name: "dead", Root: "/nonexistent"}); err != nil {
		t.Fatal(err)
	}

	msg := resumeStepCmd(resumeTarget{Name: "dead", Prune: true})()
	step, ok := msg.(resumeStepMsg)
	if !ok {
		t.Fatalf("msg type = %T, want resumeStepMsg", msg)
	}
	if step.target.Status != resumePruned {
		t.Fatalf("Status = %q, want pruned", step.target.Status)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test -run 'TestResumeJob|TestResumeStepCmd' ./...`
Expected: FAIL — `undefined: resumeJob`, `undefined: resumeStepCmd`.

- [ ] **Step 3: Write minimal implementation**

Add to `resume.go` (add `"fmt"` and `tea "github.com/charmbracelet/bubbletea"` to the import block):

```go
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test -run 'TestResumeJob|TestResumeStepCmd' ./...`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add resume.go resume_test.go
git commit -m "feat: add resume job state machine"
```

---

### Task 4: Picker restoring mode

**Files:**
- Modify: `picker.go` — add `resume` field to `pickerModel` (near `picker.go:67`); add `runPickerResuming` (after `runPicker`, `picker.go:296`); wire `Init` (`picker.go:429`); add `resumeStepMsg` case in `updateInner` (`picker.go:468`); add `resumeBanner` method and use it in `logoHeaderLines` (`picker.go:1451`).
- Test: `picker_test.go` (add cases)

**Interfaces:**
- Consumes: `resumeJob` / `resumeStepMsg` / `resumeStepCmd` (Task 3), `gatherSessions()` (`picker.go`), existing `pickerModel`, `newPickerModel`, `tea` program helpers.
- Produces: `func runPickerResuming(recreate, prune []resumeTarget) error`, `func (m pickerModel) resumeBanner() string`.

- [ ] **Step 1: Write the failing tests**

Add to `picker_test.go`:

```go
func TestResumeJobAdvancesSequentially(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	m.resume = &resumeJob{queue: []resumeTarget{
		{Name: "a", Prune: true},
		{Name: "b", Prune: true},
	}}

	n1, cmd1 := m.Update(resumeStepMsg{target: resumeTarget{Name: "a", Status: resumePruned}})
	m = n1.(pickerModel)
	if cmd1 == nil {
		t.Fatalf("expected a follow-up step cmd")
	}
	if m.resume.pos != 1 || m.resume.nPruned != 1 || m.resume.done {
		t.Fatalf("after step 1: pos=%d pruned=%d done=%v", m.resume.pos, m.resume.nPruned, m.resume.done)
	}

	n2, _ := m.Update(resumeStepMsg{target: resumeTarget{Name: "b", Status: resumePruned}})
	m = n2.(pickerModel)
	if !m.resume.done {
		t.Fatalf("expected done after final step")
	}
	if m.resume.nPruned != 2 {
		t.Fatalf("nPruned = %d, want 2", m.resume.nPruned)
	}
	if !strings.Contains(m.status, "pruned 2") {
		t.Fatalf("status = %q, want summary", m.status)
	}
}

func TestResumeBannerRendersProgress(t *testing.T) {
	m := newPickerModel([]pickerRow{
		{Kind: rowHeader, Group: "g"},
		{Kind: rowSession, Group: "g", Session: &sessionInfo{Name: "s"}},
	})
	m.resume = &resumeJob{queue: make([]resumeTarget, 2), pos: 1}

	banner := m.resumeBanner()
	if !strings.Contains(banner, "restoring") || !strings.Contains(banner, "/2") {
		t.Fatalf("banner = %q, want restoring N/2", banner)
	}

	m.resume.done = true
	if got := m.resumeBanner(); got != "" {
		t.Fatalf("done banner = %q, want empty", got)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test -run 'TestResumeJobAdvancesSequentially|TestResumeBannerRendersProgress' ./...`
Expected: FAIL — `m.resume undefined`, `m.resumeBanner undefined`.

- [ ] **Step 3: Add the `resume` field to `pickerModel`**

In `picker.go`, in the `pickerModel` struct (after `helpOpen bool`, `picker.go:98`), add:

```go
	helpOpen       bool
	resume         *resumeJob
```

- [ ] **Step 4: Add `runPickerResuming` and `resumeBanner`**

In `picker.go`, immediately after `runPicker` (ends `picker.go:296`), add:

```go
func runPickerResuming(recreate, prune []resumeTarget) error {
	queue := make([]resumeTarget, 0, len(prune)+len(recreate))
	queue = append(queue, prune...)
	queue = append(queue, recreate...)
	m := newPickerModel(gatherSessions())
	m.resume = &resumeJob{queue: queue}
	p := tea.NewProgram(m, tea.WithAltScreen())
	_, err := p.Run()
	return err
}

// resumeBanner is the live progress line shown in the logo status box while a
// resume job runs. Empty when there's no job or it's finished (the final
// summary then renders through the normal m.status path).
func (m pickerModel) resumeBanner() string {
	if m.resume == nil || m.resume.done {
		return ""
	}
	return fmt.Sprintf("restoring %d/%d…", m.resume.pos, len(m.resume.queue))
}
```

- [ ] **Step 5: Wire `Init` to kick off the first step**

In `picker.go`, replace the body of `Init` (`picker.go:429-435`) with:

```go
func (m pickerModel) Init() tea.Cmd {
	cmds := []tea.Cmd{pickerRefreshCmd(), pickerSpinnerCmd(), pickerPRRefreshCmd()}
	if m.status != "" && !m.statusSetAt.IsZero() {
		cmds = append(cmds, statusExpireCmd(m.statusSetAt))
	}
	if m.resume != nil {
		if t, ok := m.resume.nextTarget(); ok {
			cmds = append(cmds, resumeStepCmd(t))
		} else {
			m.resume.done = true
		}
	}
	return tea.Batch(cmds...)
}
```

- [ ] **Step 6: Handle `resumeStepMsg` in `updateInner`**

In `picker.go`, inside the `switch msg := msg.(type)` block in `updateInner` (starts `picker.go:468`), add a new case (place it right before `case tea.KeyMsg:`):

```go
	case resumeStepMsg:
		if m.resume == nil {
			return m, nil
		}
		m.resume.record(msg.target)
		if t, ok := m.resume.nextTarget(); ok {
			return m, resumeStepCmd(t)
		}
		m.resume.done = true
		m.status = m.resume.summary()
		m.statusErr = m.resume.nFailed > 0
		return m, nil
```

- [ ] **Step 7: Render the banner in `logoHeaderLines`**

In `picker.go`, in `logoHeaderLines` (`picker.go:1451-1458`), replace the `statusBox` construction:

```go
	statusBox := ""
	if m.status != "" {
		style := pStatus
		if m.statusErr {
			style = pStatusErr
		}
		statusBox = style.Render(m.status)
	}
```

with:

```go
	statusText := m.status
	statusErr := m.statusErr
	if banner := m.resumeBanner(); banner != "" {
		statusText = banner
		statusErr = false
	}
	statusBox := ""
	if statusText != "" {
		style := pStatus
		if statusErr {
			style = pStatusErr
		}
		statusBox = style.Render(statusText)
	}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `go test -run 'TestResumeJobAdvancesSequentially|TestResumeBannerRendersProgress' ./...`
Expected: PASS.

- [ ] **Step 9: Verify the full package still builds and passes**

Run: `go build ./... && go vet ./... && go test ./...`
Expected: build clean, vet clean, all tests PASS.

- [ ] **Step 10: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: add resume progress mode to picker"
```

---

### Task 5: `resume` subcommand wiring

**Files:**
- Modify: `commands.go` — add `resumeCommand` and register `resume` in `runCommand` (switch at `commands.go:18`).
- Modify: `main.go` — add a usage line in the Sessions block of `printUsage` (`main.go:22-29`).
- Test: `commands_test.go` (add cases)

**Interfaces:**
- Consumes: `listSessionStates()` (`session.go`), `planResume` / `availableResumeGroups` (Task 1), `runPickerResuming` (Task 4).
- Produces: `func resumeCommand(args []string) error`.

- [ ] **Step 1: Write the failing tests**

Add to `commands_test.go`:

```go
func TestResumeCommandNoSavedSessions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := resumeCommand(nil); err != nil {
		t.Fatalf("resumeCommand with no sessions = %v, want nil", err)
	}
}

func TestResumeCommandUnknownProject(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	root := t.TempDir() // group name = filepath.Base(root)
	if err := saveSessionState(&SessionState{Name: "s", Root: root}); err != nil {
		t.Fatal(err)
	}

	err := resumeCommand([]string{"definitely-not-a-group"})
	if err == nil {
		t.Fatalf("resumeCommand unknown project = nil, want error")
	}
	if !strings.Contains(err.Error(), "definitely-not-a-group") {
		t.Fatalf("error = %v, want mention of the project", err)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test -run TestResumeCommand ./...`
Expected: FAIL — `undefined: resumeCommand`.

- [ ] **Step 3: Implement `resumeCommand` and register it**

In `commands.go`, add the function (e.g. after `startSession`):

```go
func resumeCommand(args []string) error {
	fs := flag.NewFlagSet("resume", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() > 1 {
		return fmt.Errorf("resume accepts at most one project name")
	}
	filter := ""
	if fs.NArg() == 1 {
		filter = fs.Arg(0)
	}

	states, err := listSessionStates()
	if err != nil {
		return err
	}
	recreate, prune := planResume(states, filter)
	if len(recreate) == 0 && len(prune) == 0 {
		if filter != "" {
			if groups := availableResumeGroups(states); len(groups) > 0 {
				return fmt.Errorf("no saved sessions for %q (available: %s)", filter, strings.Join(groups, ", "))
			}
		}
		fmt.Println("no saved sessions to resume")
		return nil
	}
	return runPickerResuming(recreate, prune)
}
```

In `runCommand` (`commands.go:18`), add a case (next to `start`):

```go
	case "resume":
		return resumeCommand(args)
```

- [ ] **Step 4: Add the usage line**

In `main.go`, in `printUsage`, add to the Sessions block (after the `domux start` line, `main.go:23`):

```go
	fmt.Fprintf(os.Stderr, "  domux resume [PROJ]  Recreate saved sessions after a restart (all, or one project)\n")
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `go test -run TestResumeCommand ./...`
Expected: PASS.

- [ ] **Step 6: Verify the full package**

Run: `go build ./... && go vet ./... && go test ./...`
Expected: build clean, vet clean, all tests PASS.

- [ ] **Step 7: Commit**

```bash
git add commands.go main.go commands_test.go
git commit -m "feat: add domux resume subcommand"
```

---

## Self-Review

**Spec coverage:**
- Command surface (`domux resume` / `domux resume <project>`) → Task 5.
- Pure planner partitioning recreate vs. prune, group filter → Task 1.
- Group matching mirrors `gatherSessions` (workspace shares main group), case-insensitive → Task 1 (`resumeGroup`, tests).
- Unknown project → friendly error + available groups; empty states → friendly exit → Task 5.
- Skip + prune dead-root states (remove json + legacy files) → Task 2 (`executeResumeStep` prune branch).
- Recreate bare detached session at saved root, skip already-running, re-pin root → Task 2.
- Sequential execution driven by the picker → Tasks 3 (job) + 4 (Init/Update wiring).
- Progress banner in the logo status box + self-populating live list (existing refresh) + final summary on the 5s TTL → Task 4.
- "Just the shell session" depth (no worktree setup, no agents/servers) → executor only calls `createTmuxSession` + `setSessionRoot`; no setup invoked (Task 2).
- Failure isolation (one bad target doesn't abort the rest) → `executeResumeStep` captures `Err`; `record` tallies `nFailed`; loop continues (Tasks 2–4).

**Placeholder scan:** No TBD/TODO; every code and test step shows full content. No "handle errors appropriately" hand-waving — error paths are concrete.

**Type consistency:** `resumeTarget` (fields `Name/Root/Group/Prune/Status/Err`), `resumeStatus` consts (`resumeRecreated`/`resumeRunning`/`resumePruned`), `resumeJob` (fields `queue/pos/done/nRecreated/nRunning/nPruned/nFailed`), `resumeStepMsg{target}`, and functions `resumeGroup`/`planResume`/`sortResumeTargets`/`availableResumeGroups`/`executeResumeStep`/`resumeStepCmd`/`runPickerResuming`/`resumeBanner`/`resumeCommand` are referenced with identical names and signatures across all tasks. `dirExists`/`fileExists` are reused from `install.go`, not redefined.
