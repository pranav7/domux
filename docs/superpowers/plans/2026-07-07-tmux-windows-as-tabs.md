# Tmux Windows as Tabs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface each tmux window as a first-class, jumpable row in the domux switcher, and let any session grow by adding a named window (`w`), giving no-git-root sessions the multiplication mechanism worktree provisioning (`+`) can't.

**Architecture:** Extend `gatherSessions` to enumerate each session's windows (with per-window cwd) into a new `windowInfo` slice on `sessionInfo`. Add a `rowWindow` row kind rendered under the session row (only when >1 window). Per-window AI status comes from a new `aggregateAIStatesByWindow` that buckets the existing `agent:window_pane` keys by window index instead of collapsing them; per-window recap resolves by that window's cwd. Enter on a window row carries a `session:window_index` target through the existing deferred-attach path; `w` reuses the label-input overlay to create a named window.

**Tech Stack:** Go 1.22, single `package main`. bubbletea/lipgloss TUI. tmux via `exec.Command`. No new dependencies.

## Global Constraints

- Go 1.22, single-package layout (`package main`), no module subdirs.
- Atomic writes for on-disk state: write `path + ".tmp"` then `os.Rename` (N/A here — this feature writes no new persisted state).
- Tmux interop is `exec.Command` only — no library.
- `normalizeAIState` is the single source of truth for AI-state canonicalization — go through it, never re-canonicalize inline.
- `state.AI` keys are `agent:window_pane` (e.g. `claude:2_0`); the `window_pane` half uses `_` as separator. Do not change this format.
- Match surrounding code style: table-driven tests in the `picker_test.go` / `session_test.go` idiom; Catppuccin Mocha palette variables already declared in `tui.go` / `picker.go`.
- Build/test commands: `go build`, `go test ./...`, `go vet ./...`.
- **Key bindings (verified in current `picker.go`):** `+` = provision worktree (NOT `p`); `P` = preview popup; `n` = name/label; `tab` = show/hide details. The new window key is **`w`**.

---

## File Structure

- `picker.go` (modify) — all TUI changes: `windowInfo` type, `rowWindow` kind, window enumeration in `gatherSessions`, `rowsFromEntries` window rows, `renderWindow`, cursor/selection over window rows, `w` key + `newWindowInSession`, `selectRow` window target, help overlay line, `+` guidance message.
- `session.go` (modify) — `aggregateAIStatesByWindow(state *SessionState) map[int]AIStates`.
- `commands.go` (modify) — none required; `attachTmuxSession` already accepts a `session:window` target verbatim. (No change; noted so the implementer doesn't go looking.)
- `session_test.go` (modify/create if absent) — tests for `aggregateAIStatesByWindow`.
- `picker_test.go` (modify) — tests for row flattening, `renderWindow`, selection target, `w` flow, guidance message.

**Task ordering rationale:** data model first (Task 1: per-window AI aggregation, pure/testable in isolation), then enumeration (Task 2), then rendering (Task 3), then navigation+selection (Task 4), then the `w` create action (Task 5), then polish/help/guidance (Task 6). Each task ends green and committable.

---

### Task 1: Per-window AI-state aggregation

**Files:**
- Modify: `session.go` (add `aggregateAIStatesByWindow` near `aggregateAIStatesFromSession`, ~line 451)
- Test: `session_test.go`

**Interfaces:**
- Consumes: existing `AIStates` struct (`session.go:35` — fields `Claude, Codex, ClaudeLabel, CodexLabel string`), `inferAgentFromAIKey` (`session.go:625`), `inferAgentFromAIValue`, `normalizeAIState`, `mergeAIState` (`session.go:602`), `aiWorkingLabelForAgent` (`session.go:562`).
- Produces: `func aggregateAIStatesByWindow(state *SessionState) map[int]AIStates` — maps tmux window index → aggregated AI state for that window. Windows with no AI keys do not appear in the map. Keys whose `window_pane` half has no parseable leading integer are skipped.

- [ ] **Step 1: Write the failing test**

Add to `session_test.go`:

```go
func TestAggregateAIStatesByWindow(t *testing.T) {
	state := &SessionState{AI: map[string]string{
		"claude:1_0": "CLAUDING",
		"claude:2_0": "WAITING",
		"codex:2_0":  "CODEXING",
		"claude:2_1": "CLAUDING", // second pane in window 2; WAITING must still win
	}}

	got := aggregateAIStatesByWindow(state)

	if got[1].Claude != "CLAUDING" {
		t.Errorf("window 1 Claude = %q, want CLAUDING", got[1].Claude)
	}
	if got[2].Claude != "WAITING" {
		t.Errorf("window 2 Claude = %q, want WAITING (WAITING outranks CLAUDING)", got[2].Claude)
	}
	if got[2].Codex != "CODEXING" {
		t.Errorf("window 2 Codex = %q, want CODEXING", got[2].Codex)
	}
	if _, ok := got[3]; ok {
		t.Errorf("window 3 should be absent, got %+v", got[3])
	}
}

func TestAggregateAIStatesByWindowNilAndEmpty(t *testing.T) {
	if got := aggregateAIStatesByWindow(nil); len(got) != 0 {
		t.Errorf("nil state = %+v, want empty map", got)
	}
	if got := aggregateAIStatesByWindow(&SessionState{}); len(got) != 0 {
		t.Errorf("empty state = %+v, want empty map", got)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestAggregateAIStatesByWindow ./...`
Expected: FAIL — `undefined: aggregateAIStatesByWindow`.

- [ ] **Step 3: Write minimal implementation**

Add to `session.go` immediately after `aggregateAIStatesFromSession` (ends ~line 481):

```go
// aggregateAIStatesByWindow buckets state.AI entries by tmux window index
// instead of collapsing every pane into one session-level status. The AI keys
// are "agent:window_pane" (e.g. "claude:2_0"); the leading integer of the
// window_pane half is the window index. WAITING wins over COMPACTING wins over
// the plain working states, per mergeAIState — same precedence used
// session-wide. Windows with no AI keys are absent from the returned map.
func aggregateAIStatesByWindow(state *SessionState) map[int]AIStates {
	out := map[int]AIStates{}
	if state == nil {
		return out
	}
	for key, value := range state.AI {
		_, paneKey, ok := strings.Cut(key, ":")
		if !ok {
			continue
		}
		winStr, _, ok := strings.Cut(paneKey, "_")
		if !ok {
			continue
		}
		win, err := strconv.Atoi(winStr)
		if err != nil {
			continue
		}
		agent := inferAgentFromAIKey(key)
		if agent == "" {
			agent = inferAgentFromAIValue(value)
		}
		if agent == "" {
			agent = "claude"
		}
		value = normalizeAIState(agent, value)
		s := out[win]
		switch agent {
		case "codex":
			s.Codex = mergeAIState(s.Codex, value)
		default:
			s.Claude = mergeAIState(s.Claude, value)
		}
		out[win] = s
	}
	// Assign working labels per window, mirroring aggregateAIStatesFromSession.
	for win, s := range out {
		usedLabels := map[string]bool{}
		if s.Claude == "CLAUDING" {
			s.ClaudeLabel = aiWorkingLabelForAgent(state, "claude", usedLabels)
			usedLabels[s.ClaudeLabel] = true
		}
		if s.Codex == "CODEXING" {
			s.CodexLabel = aiWorkingLabelForAgent(state, "codex", usedLabels)
		}
		out[win] = s
	}
	return out
}
```

Confirm `session.go` already imports `strconv` and `strings`. If `strconv` is missing, add it to the import block.

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run TestAggregateAIStatesByWindow ./...`
Expected: PASS (both tests).

- [ ] **Step 5: Verify no import breakage & vet**

Run: `go vet ./... && go build`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add session.go session_test.go
git commit -m "feat: aggregate AI states per tmux window"
```

---

### Task 2: Enumerate windows in `gatherSessions`

**Files:**
- Modify: `picker.go` — add `windowInfo` type near `sessionInfo` (~line 34); add `Windows []windowInfo` field to `sessionInfo`; replace the `list-windows` count call (~line 2356) with enumeration.
- Test: `picker_test.go`

**Interfaces:**
- Consumes: `aggregateAIStatesByWindow` (Task 1); existing `readClaudeSessions()`, `bestLiveSession` (`recap.go:102`), `recapForSession` (`recap.go:128`), `recapVisibleAfterClear` (`recap.go:154`), `loadSessionStateWithLegacy`.
- Produces:
  ```go
  type windowInfo struct {
      Index       int
      Name        string
      Active      bool
      Path        string
      Claude      string
      Codex       string
      ClaudeLabel string
      CodexLabel  string
      Recap       string
  }
  ```
  and `sessionInfo.Windows []windowInfo`. `parseWindowLines(out string) []windowInfo` — pure parser for the `list-windows` tab-separated output, testable without tmux.

- [ ] **Step 1: Write the failing test for the pure parser**

Add to `picker_test.go`:

```go
func TestParseWindowLines(t *testing.T) {
	out := "1\tprod uk\t0\t/Users/x/projects/audrey\n" +
		"2\tmerge queue\t1\t/Users/x/projects/audrey\n"
	got := parseWindowLines(out)
	if len(got) != 2 {
		t.Fatalf("got %d windows, want 2", len(got))
	}
	if got[0].Index != 1 || got[0].Name != "prod uk" || got[0].Active {
		t.Errorf("window 0 = %+v, want {Index:1 Name:%q Active:false}", got[0], "prod uk")
	}
	if got[1].Index != 2 || got[1].Name != "merge queue" || !got[1].Active {
		t.Errorf("window 1 = %+v, want {Index:2 Name:%q Active:true}", got[1], "merge queue")
	}
	if got[1].Path != "/Users/x/projects/audrey" {
		t.Errorf("window 1 Path = %q", got[1].Path)
	}
}

func TestParseWindowLinesEmpty(t *testing.T) {
	if got := parseWindowLines(""); len(got) != 0 {
		t.Errorf("empty output = %+v, want no windows", got)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestParseWindowLines ./...`
Expected: FAIL — `undefined: parseWindowLines` and `undefined: windowInfo`.

- [ ] **Step 3: Add the `windowInfo` type and `Windows` field**

In `picker.go`, add after the `taskInfo` struct (`picker.go:58`):

```go
type windowInfo struct {
	Index       int
	Name        string
	Active      bool
	Path        string // window's active-pane cwd
	Claude      string
	Codex       string
	ClaudeLabel string
	CodexLabel  string
	Recap       string
}
```

Add to the `sessionInfo` struct (after `Tasks []taskInfo`, `picker.go:48`):

```go
	Windows []windowInfo
```

- [ ] **Step 4: Write the pure parser**

Add near `gatherSessions` in `picker.go`:

```go
// parseWindowLines parses `tmux list-windows -F
// "#{window_index}\t#{window_name}\t#{window_active}\t#{pane_current_path}"`
// output into windowInfo values (AI/Recap fields left zero — filled by caller).
func parseWindowLines(out string) []windowInfo {
	var windows []windowInfo
	for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "\t", 4)
		if len(parts) < 4 {
			continue
		}
		idx, err := strconv.Atoi(parts[0])
		if err != nil {
			continue
		}
		windows = append(windows, windowInfo{
			Index:  idx,
			Name:   parts[1],
			Active: parts[2] == "1",
			Path:   parts[3],
		})
	}
	return windows
}
```

- [ ] **Step 5: Run parser test to verify it passes**

Run: `go test -run TestParseWindowLines ./...`
Expected: PASS.

- [ ] **Step 6: Wire enumeration into `gatherSessions`**

In `picker.go`, replace the window-count block (currently ~lines 2356-2359):

```go
		winOut, err := exec.Command("tmux", "list-windows", "-t", sess).Output()
		if err == nil {
			info.Windows = len(strings.Split(strings.TrimSpace(string(winOut)), "\n"))
		}
```

with enumeration + per-window AI + per-window recap:

```go
		winOut, err := exec.Command("tmux", "list-windows", "-t", sess, "-F",
			"#{window_index}\t#{window_name}\t#{window_active}\t#{pane_current_path}").Output()
		if err == nil {
			windows := parseWindowLines(string(winOut))
			winStates := aggregateAIStatesByWindow(state)
			for i := range windows {
				w := &windows[i]
				if s, ok := winStates[w.Index]; ok {
					w.Claude, w.Codex = s.Claude, s.Codex
					w.ClaudeLabel, w.CodexLabel = s.ClaudeLabel, s.CodexLabel
				}
				if w.Path != "" {
					if best, ok := bestLiveSession(liveClaude, w.Path); ok {
						recap, recapTime := recapForSession(best)
						if recapVisibleAfterClear(recapTime, state.RecapClearedAt) {
							w.Recap = recap
						}
					}
				}
			}
			info.Windows = windows
		}
```

Because `info.Windows` changes type from `int` to `[]windowInfo`, remove the old `Windows int` field (it was added in Step 3 as `Windows []windowInfo`, so the old `int` field at `picker.go:43` must be deleted). Search for other readers of `info.Windows`/`.Windows` (`grep -n "\.Windows" picker.go`) and confirm none rely on the int count; if any do, replace with `len(....Windows)`.

- [ ] **Step 7: Run full build & tests**

Run: `go build && go test ./...`
Expected: build clean; all tests pass. If a compile error points at a former `int` use of `Windows`, fix it to `len(x.Windows)`.

- [ ] **Step 8: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: enumerate tmux windows with per-window AI status and recap"
```

---

### Task 3: Render window sub-rows under sessions

**Files:**
- Modify: `picker.go` — add `rowWindow` to the `rowKind` const block (`picker.go:20-26`); add `Window *windowInfo` field to `pickerRow` (`picker.go:60-65`); append window rows in `rowsFromEntries` (`picker.go:2430`); add `renderWindow`; dispatch it in `renderRow` (`picker.go:2057-2074`).
- Test: `picker_test.go`

**Interfaces:**
- Consumes: `windowInfo` (Task 2); existing `renderAIBadges(claude, codex, claudeLabel, codexLabel string, spinnerFrame int) string` (`picker.go:2226`); `m.showDetails`; `wrapWords`; palette styles `pRecapIcon`, `pRecapText` (`picker.go:167-168`); `teal`, `overlay0` colors.
- Produces: `rowWindow rowKind`; `pickerRow.Window *windowInfo`; `func (m pickerModel) renderWindow(row pickerRow, selected bool) string`.

- [ ] **Step 1: Write the failing test for row flattening**

Add to `picker_test.go`:

```go
func TestRowsFromEntriesWindowRows(t *testing.T) {
	// >1 window → one rowWindow per window, under the session, above tasks.
	multi := &sessionInfo{
		Name: "domux", Path: "/p",
		Windows: []windowInfo{{Index: 1, Name: "a"}, {Index: 2, Name: "b"}},
		Tasks:   []taskInfo{{Title: "t1"}},
	}
	rows := rowsFromEntries([]groupEntry{{group: "domux", session: multi}})

	var kinds []rowKind
	for _, r := range rows {
		kinds = append(kinds, r.Kind)
	}
	// header, session, window, window, task
	want := []rowKind{rowHeader, rowSession, rowWindow, rowWindow, rowTask}
	if len(kinds) != len(want) {
		t.Fatalf("kinds = %v, want %v", kinds, want)
	}
	for i := range want {
		if kinds[i] != want[i] {
			t.Fatalf("kinds = %v, want %v", kinds, want)
		}
	}
}

func TestRowsFromEntriesSingleWindowNoWindowRows(t *testing.T) {
	single := &sessionInfo{
		Name: "solo", Path: "/p",
		Windows: []windowInfo{{Index: 1, Name: "a"}},
	}
	rows := rowsFromEntries([]groupEntry{{group: "solo", session: single}})
	for _, r := range rows {
		if r.Kind == rowWindow {
			t.Fatalf("single-window session must not emit rowWindow, got rows %+v", rows)
		}
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestRowsFromEntries ./...`
Expected: FAIL — `undefined: rowWindow` and `pickerRow.Window`.

- [ ] **Step 3: Add the row kind and field**

In `picker.go` const block (`picker.go:20-26`), add `rowWindow` after `rowTask`:

```go
const (
	rowHeader rowKind = iota
	rowSpacer
	rowSession
	rowTask
	rowWindow
	rowRule
)
```

Add to `pickerRow` (`picker.go:60-65`):

```go
	Window  *windowInfo
```

- [ ] **Step 4: Append window rows in `rowsFromEntries`**

In `picker.go`, in the loop body of `rowsFromEntries` (`picker.go:2430-2433`), insert window rows between the session row and task rows:

```go
		rows = append(rows, pickerRow{Kind: rowSession, Group: e.group, Session: e.session})
		if len(e.session.Windows) > 1 {
			for i := range e.session.Windows {
				rows = append(rows, pickerRow{Kind: rowWindow, Group: e.group, Window: &e.session.Windows[i]})
			}
		}
		for i := range e.session.Tasks {
			rows = append(rows, pickerRow{Kind: rowTask, Group: e.group, Task: &e.session.Tasks[i]})
		}
```

- [ ] **Step 5: Run flattening tests to verify they pass**

Run: `go test -run TestRowsFromEntries ./...`
Expected: PASS.

- [ ] **Step 6: Write the failing test for `renderWindow`**

Add to `picker_test.go`:

```go
func TestRenderWindowActiveAndBadge(t *testing.T) {
	m := pickerModel{width: 120}
	active := pickerRow{Kind: rowWindow, Window: &windowInfo{
		Index: 2, Name: "merge queue", Active: true, Claude: "WAITING",
	}}
	out := m.renderWindow(active, false)
	if !strings.Contains(out, "2") || !strings.Contains(out, "merge queue") {
		t.Errorf("renderWindow missing index/name: %q", out)
	}
}

func TestRenderWindowRecapGatedByDetails(t *testing.T) {
	win := &windowInfo{Index: 1, Name: "a", Recap: "reshaped the pipeline"}

	off := pickerModel{width: 120, showDetails: false}
	if strings.Contains(off.renderWindow(pickerRow{Kind: rowWindow, Window: win}, false), "reshaped the pipeline") {
		t.Errorf("recap must be hidden when showDetails is off")
	}

	on := pickerModel{width: 120, showDetails: true}
	if !strings.Contains(on.renderWindow(pickerRow{Kind: rowWindow, Window: win}, false), "reshaped the pipeline") {
		t.Errorf("recap must show when showDetails is on")
	}
}
```

- [ ] **Step 7: Run test to verify it fails**

Run: `go test -run TestRenderWindow ./...`
Expected: FAIL — `undefined: (pickerModel).renderWindow`.

- [ ] **Step 8: Implement `renderWindow` and dispatch it**

Add `renderWindow` to `picker.go` (next to `renderTask`, `picker.go:2256`):

```go
// renderWindow renders a tmux window as an indented, jumpable sub-row under its
// session. Layout mirrors renderTask (indent 8, aligned with the recap ※
// column). The active window's glyph is highlighted; the AI badge reuses
// renderAIBadges; the recap hangs under the row and is gated by showDetails —
// the same tab toggle used for session recaps.
func (m pickerModel) renderWindow(row pickerRow, selected bool) string {
	w := row.Window
	glyphStyle := lipgloss.NewStyle().Foreground(overlay0)
	nameStyle := lipgloss.NewStyle().Foreground(overlay0)
	if w.Active {
		glyphStyle = lipgloss.NewStyle().Foreground(teal).Bold(true)
		nameStyle = lipgloss.NewStyle().Foreground(teal)
	}
	if selected {
		nameStyle = nameStyle.Bold(true)
	}

	var line strings.Builder
	// Prefix: cursor arrow when selected, else blank; 8-col indent to align with
	// tasks/recap.
	if selected {
		line.WriteString("      " + pCursor.Render("›") + " ")
	} else {
		line.WriteString("        ")
	}
	line.WriteString(glyphStyle.Render("▸") + " ")
	line.WriteString(nameStyle.Render(fmt.Sprintf("%d · %s", w.Index, w.Name)))
	line.WriteString(renderAIBadges(w.Claude, w.Codex, w.ClaudeLabel, w.CodexLabel, m.spinnerFrame))

	if m.showDetails && w.Recap != "" {
		const indent = "          " // 10 cols, under the window name
		avail := m.width - lipgloss.Width(indent)
		if avail < 8 {
			avail = 8
		}
		for i, seg := range wrapWords(w.Recap, avail) {
			if i == 0 {
				line.WriteString("\n" + indent + pRecapIcon.Render("※") + " " + pRecapText.Render(seg))
			} else {
				line.WriteString("\n" + indent + "  " + pRecapText.Render(seg))
			}
		}
	}
	return line.String()
}
```

Dispatch it in `renderRow` (`picker.go:2057-2074`). The current function switches on `row.Kind`; add a `rowWindow` case alongside the existing `rowSession`/`rowTask` cases:

```go
	case rowWindow:
		return m.renderWindow(row, selected)
```

(Place it next to `case rowTask: return m.renderTask(row, selected)`. Read `renderRow` first to match its exact switch shape.)

- [ ] **Step 9: Run render tests to verify they pass**

Run: `go test -run TestRenderWindow ./...`
Expected: PASS.

- [ ] **Step 10: Full build & test**

Run: `go build && go test ./... && go vet ./...`
Expected: all green.

- [ ] **Step 11: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: render tmux window sub-rows with per-window status and recap"
```

---

### Task 4: Make window rows selectable and jumpable

**Files:**
- Modify: `picker.go` — `isSelectablePickerRow` (`picker.go:459`), `selectRow` (`picker.go:930`), and a new `windowTarget` helper. Add a `selectedRow` accessor for the `w` action in Task 5.
- Test: `picker_test.go`

**Interfaces:**
- Consumes: `pickerRow` with `Window` (Task 3); `attachTmuxSession` (`commands.go:267`, already accepts `session:window`); `switchSession` (`picker.go:2276`).
- Produces: updated `isSelectablePickerRow` (now true for `rowWindow`); `selectRow` sets `m.selected = "<session>:<index>"` for window rows; `func windowSessionName(rows []pickerRow, win *windowInfo) string` — resolves a window row's parent session by scanning backwards. `selectRow` needs the parent session name, so window rows must carry it.

**Design note on parent-session resolution:** a `rowWindow` currently carries only `*windowInfo`, not its session name. Add a `Session *sessionInfo` reference to the window row when flattening (cheapest, avoids backward scans). Update Task 3's flattening line accordingly.

- [ ] **Step 1: Carry the parent session on window rows**

In `picker.go`, `rowsFromEntries` (the window loop added in Task 3), set `Session` too so a window row knows its session:

```go
		if len(e.session.Windows) > 1 {
			for i := range e.session.Windows {
				rows = append(rows, pickerRow{
					Kind:    rowWindow,
					Group:   e.group,
					Session: e.session,
					Window:  &e.session.Windows[i],
				})
			}
		}
```

(`pickerRow.Session` already exists — `picker.go:63`.)

- [ ] **Step 2: Write the failing test**

Add to `picker_test.go`:

```go
func TestSelectRowWindowTarget(t *testing.T) {
	sess := &sessionInfo{Name: "domux", Path: "/p",
		Windows: []windowInfo{{Index: 1, Name: "a"}, {Index: 2, Name: "b"}}}
	winRow := pickerRow{Kind: rowWindow, Session: sess, Window: &sess.Windows[1]}

	m := pickerModel{}
	updated, _ := m.selectRow(winRow)
	pm := updated.(pickerModel)
	if pm.selected != "domux:2" {
		t.Errorf("selected = %q, want domux:2", pm.selected)
	}
}

func TestSelectRowSessionTargetUnchanged(t *testing.T) {
	sess := &sessionInfo{Name: "domux"}
	m := pickerModel{}
	updated, _ := m.selectRow(pickerRow{Kind: rowSession, Session: sess})
	pm := updated.(pickerModel)
	if pm.selected != "domux" {
		t.Errorf("selected = %q, want domux", pm.selected)
	}
}

func TestWindowRowIsSelectable(t *testing.T) {
	if !isSelectablePickerRow(pickerRow{Kind: rowWindow, Window: &windowInfo{Index: 1}}) {
		t.Errorf("rowWindow should be selectable")
	}
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `go test -run 'TestSelectRow|TestWindowRowIsSelectable' ./...`
Expected: FAIL — window row not selectable; `selected` empty for window row.

- [ ] **Step 4: Update `isSelectablePickerRow`**

Replace `picker.go:459-461`:

```go
func isSelectablePickerRow(row pickerRow) bool {
	return (row.Kind == rowSession && row.Session != nil) ||
		(row.Kind == rowWindow && row.Window != nil && row.Session != nil)
}
```

- [ ] **Step 5: Update `selectRow`**

Replace `selectRow` (`picker.go:930-936`):

```go
func (m pickerModel) selectRow(row pickerRow) (tea.Model, tea.Cmd) {
	switch row.Kind {
	case rowSession:
		if row.Session != nil {
			m.selected = row.Session.Name
			return m, tea.Quit
		}
	case rowWindow:
		if row.Session != nil && row.Window != nil {
			m.selected = fmt.Sprintf("%s:%d", row.Session.Name, row.Window.Index)
			return m, tea.Quit
		}
	}
	return m, nil
}
```

`attachTmuxSession` (`commands.go:267`) already passes `m.selected` verbatim to `switch-client -t` / `attach-session -t`, both of which accept a `session:window` target — no change needed there. (`clearWaitingState(name)` at the top of `attachTmuxSession` receives `"domux:2"`; verify it tolerates the suffix — read `clearWaitingState`. If it does an exact session-name match, split on the first `:` inside it or before calling. Add a test only if you change it.)

- [ ] **Step 6: Verify `clearWaitingState` tolerates the target**

Run: `grep -n "func clearWaitingState" *.go` then read it. If it globs by exact session name and would miss with a `:index` suffix, trim the suffix at the call site instead:

```go
func attachTmuxSession(name string) error {
	sessionName, _, _ := strings.Cut(name, ":")
	clearWaitingState(sessionName)
	...
```

and keep passing the full `name` to the tmux attach args. If `clearWaitingState` already tolerates it, leave as-is. Either way, `go build` must stay clean.

- [ ] **Step 7: Run selection tests to verify they pass**

Run: `go test -run 'TestSelectRow|TestWindowRowIsSelectable' ./...`
Expected: PASS.

- [ ] **Step 8: Full build & test**

Run: `go build && go test ./... && go vet ./...`
Expected: all green. `moveCursor`/`clampCursor`/`g`/`G` now naturally stop on window rows because they gate on `isSelectablePickerRow` — no change needed. Manually sanity-check that cursor navigation lands on window rows.

- [ ] **Step 9: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: select and jump to a specific tmux window from the switcher"
```

---

### Task 5: `w` key — create a named window on any session

**Files:**
- Modify: `picker.go` — add `w` case in the main key switch (near `picker.go:832`, the `+` case); add a window-name input flow reusing the label-input overlay state; add `newWindowInSession`; extend `applyPickerAction` for `Action: "window"`.
- Test: `picker_test.go`

**Interfaces:**
- Consumes: `m.labelInput`, `m.labelEditing`, `m.labelTarget` (existing overlay state, `picker.go:73-75`); `pickerActionMsg{Action, Session, Value, Err}` (`picker.go:106`); `applyPickerAction` (`picker.go:1121`); `selectedSession` (`picker.go:938`) and the window-aware selection from Task 4.
- Produces: `func newWindowInSession(session, name, cwd string) error`; a distinct overlay mode so `w`'s input is not confused with `n`'s label edit. Add a bool `windowEditing` and reuse `labelInput`, OR reuse `labelEditing` with a mode discriminator. This plan uses a **separate `windowNaming` bool + `windowTarget`/`windowCwd` strings** to keep the label path untouched.

- [ ] **Step 1: Write the failing test for the tmux command builder**

`newWindowInSession` shells out, so test the *arg construction* via a pure helper. Add to `picker_test.go`:

```go
func TestNewWindowArgs(t *testing.T) {
	got := newWindowArgs("domux", "merge queue", "/Users/x/projects/audrey")
	want := []string{"new-window", "-t", "domux", "-n", "merge queue", "-c", "/Users/x/projects/audrey"}
	if len(got) != len(want) {
		t.Fatalf("args = %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("args = %v, want %v", got, want)
		}
	}
}

func TestNewWindowArgsNoCwd(t *testing.T) {
	got := newWindowArgs("domux", "scratch", "")
	want := []string{"new-window", "-t", "domux", "-n", "scratch"}
	if len(got) != len(want) {
		t.Fatalf("args = %v, want %v", got, want)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestNewWindowArgs ./...`
Expected: FAIL — `undefined: newWindowArgs`.

- [ ] **Step 3: Implement the arg builder and the command**

Add to `picker.go` (near the other tmux action helpers):

```go
// newWindowArgs builds the `tmux new-window` argv for creating a named window in
// session at cwd. cwd is omitted when empty (tmux inherits the session default).
func newWindowArgs(session, name, cwd string) []string {
	args := []string{"new-window", "-t", session, "-n", name}
	if cwd != "" {
		args = append(args, "-c", cwd)
	}
	return args
}

// newWindowInSession creates a new named tmux window in session, rooted at cwd.
func newWindowInSession(session, name, cwd string) error {
	out, err := exec.Command("tmux", newWindowArgs(session, name, cwd)...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux new-window -t %s: %w: %s", session, err, strings.TrimSpace(string(out)))
	}
	return nil
}
```

- [ ] **Step 4: Run arg-builder tests to verify they pass**

Run: `go test -run TestNewWindowArgs ./...`
Expected: PASS.

- [ ] **Step 5: Add the window-naming overlay state**

Add fields to `pickerModel` (near `labelTarget`, `picker.go:75`):

```go
	windowNaming bool
	windowTarget string // session name to add the window to
	windowCwd    string // cwd for the new window
```

- [ ] **Step 6: Bind `w` and handle its input**

In the main key switch (`picker.go`), add a `w` case next to the `+` case (`picker.go:832`):

```go
		case "w":
			m.startWindowNaming()
			return m, nil
```

Add `startWindowNaming` (next to `startLabelEdit`, `picker.go:1028`):

```go
func (m *pickerModel) startWindowNaming() {
	session := m.selectedSession()
	cwd := ""
	target := ""
	if session != nil {
		target = session.Name
		cwd = session.Path
	} else if row := m.rows[m.visible[m.cursor]]; row.Kind == rowWindow && row.Session != nil {
		target = row.Session.Name
		if row.Window != nil {
			cwd = row.Window.Path
		}
	}
	if target == "" {
		return
	}
	m.windowNaming = true
	m.windowTarget = target
	m.windowCwd = cwd
	m.labelInput.SetValue("")
}
```

Add the input handler block. Place it right **before** the `if m.labelEditing {` block (`picker.go:650`) so it intercepts keys first:

```go
		if m.windowNaming {
			switch key {
			case "ctrl+c":
				return m, tea.Quit
			case "esc":
				m.windowNaming = false
				m.windowTarget = ""
				m.windowCwd = ""
				m.labelInput.SetValue("")
				return m, nil
			case "enter":
				target := m.windowTarget
				cwd := m.windowCwd
				name := strings.TrimSpace(m.labelInput.Value())
				m.windowNaming = false
				m.windowTarget = ""
				m.windowCwd = ""
				m.labelInput.SetValue("")
				if target == "" || name == "" {
					return m, nil
				}
				m.status = fmt.Sprintf("adding window %q to %s", name, target)
				m.statusErr = false
				return m, func() tea.Msg {
					return pickerActionMsg{
						Action:  "window",
						Session: target,
						Value:   name,
						Err:     newWindowInSession(target, name, cwd),
					}
				}
			case "backspace":
				v := m.labelInput.Value()
				if len(v) > 0 {
					m.labelInput.SetValue(v[:len(v)-1])
				}
				return m, nil
			default:
				if len(key) == 1 && key[0] >= 32 && key[0] < 127 {
					m.labelInput.SetValue(m.labelInput.Value() + key)
				}
				return m, nil
			}
		}
```

- [ ] **Step 7: Render the naming overlay**

The `View` method (`picker.go:1362`) renders `renderLabelOverlay` when `m.labelEditing`. Add a parallel branch for `m.windowNaming`. In `View`, after the `if m.labelEditing {` branch (`picker.go:1373-1375`):

```go
	if m.windowNaming {
		return m.renderWindowNamingOverlay()
	}
```

Add `renderWindowNamingOverlay` modeled on `renderLabelOverlay` (`picker.go:1315`) — read that function and mirror it, swapping the prompt text to `"new window name for <session>"`. Reuse `m.labelInput.View()` for the input line.

- [ ] **Step 8: Handle `Action: "window"` in `applyPickerAction`**

In `applyPickerAction` error switch (`picker.go:1134-1150`) add:

```go
			case "window":
				m.status = fmt.Sprintf("add window to %s failed: %v", msg.Session, msg.Err)
```

In the success switch (after `picker.go:1157`, alongside `case "provision"`) add:

```go
		case "window":
			m.status = fmt.Sprintf("added window %q to %s", msg.Value, msg.Session)
```

(The 2s refresh tick re-runs `gatherSessions` and picks up the new window; no manual row insertion needed.)

- [ ] **Step 9: Write the input-flow test**

Add to `picker_test.go`:

```go
func TestWindowNamingFlow(t *testing.T) {
	sess := &sessionInfo{Name: "domux", Path: "/p", Windows: []windowInfo{{Index: 1}, {Index: 2}}}
	m := pickerModel{
		rows:    []pickerRow{{Kind: rowSession, Session: sess}},
		visible: []int{0},
		cursor:  0,
	}
	m.labelInput = newLabelInput() // if a constructor exists; otherwise set via textinput.New()
	m.startWindowNaming()
	if !m.windowNaming || m.windowTarget != "domux" || m.windowCwd != "/p" {
		t.Fatalf("startWindowNaming state = naming:%v target:%q cwd:%q", m.windowNaming, m.windowTarget, m.windowCwd)
	}
}
```

If there is no `newLabelInput` constructor, initialise `m.labelInput` the same way `newPickerModel` does (read `picker.go:392` for the exact `textinput` setup) or drop that line if `startWindowNaming` doesn't touch the input beyond `SetValue`.

- [ ] **Step 10: Run the flow test**

Run: `go test -run TestWindowNamingFlow ./...`
Expected: PASS.

- [ ] **Step 11: Full build & test**

Run: `go build && go test ./... && go vet ./...`
Expected: all green.

- [ ] **Step 12: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: press w to add a named tmux window to any session"
```

---

### Task 6: Help overlay, footer hint, and `+` guidance message

**Files:**
- Modify: `picker.go` — `renderHelpOverlay` (`picker.go:1346-1348`); `provisionInFocusedGroup` guidance message (`picker.go:1057-1060`).
- Test: `picker_test.go` (existing `picker_test.go:968` asserts the "no git root" message — update it).

**Interfaces:**
- Consumes: existing help overlay `bind`/`join` helpers; `provisionInFocusedGroup` (`picker.go:1055`).
- Produces: updated help text with `w new window`; updated no-git-root guidance.

- [ ] **Step 1: Update the existing guidance-message test**

The test at `picker_test.go:968` currently asserts the status contains `"no git root"`. Update it to assert the new guidance:

```go
	if pm.status == "" || !strings.Contains(pm.status, "press w to add a window") {
		t.Fatalf("expected guidance to mention 'press w to add a window', got %q", pm.status)
	}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test -run TestPicker ./...` (find the exact test name around `picker_test.go:968` — likely `TestProvision...`; run that one).
Expected: FAIL — status still says only "no git root for this row".

- [ ] **Step 3: Update the guidance message**

In `provisionInFocusedGroup` (`picker.go:1057-1060`):

```go
	if session == nil || session.Root == "" {
		m.status = "no git root — press w to add a window"
		m.statusErr = true
		return nil
	}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test -run TestPicker ./...` (the specific test from Step 2).
Expected: PASS.

- [ ] **Step 5: Add `w` to the help overlay**

In `renderHelpOverlay`, the SESSION block (`picker.go:1346-1348`), add `w new window` to the first SESSION line:

```go
	b.WriteString(catS.Render("SESSION") + "\n")
	b.WriteString("  " + join(bind("⏎", "switch"), bind("+", "new"), bind("w", "new window"), bind("D", "close/delete")) + "\n")
	b.WriteString("  " + join(bind("n", "name"), bind("c", "clear"), bind("r", "reset"), bind("s", "server")) + "\n\n")
```

- [ ] **Step 6: Full build, test, vet**

Run: `go build && go test ./... && go vet ./...`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add picker.go picker_test.go
git commit -m "feat: document w (new window) in help and point + at it on no-git-root rows"
```

---

## Manual verification (after all tasks)

Since this is a TUI over live tmux, do an end-to-end pass (this repo's `verify` skill / `run` skill drives the built binary):

1. `go build` → `./domux` in a tmux session with multiple windows.
2. Confirm a session with >1 window shows window sub-rows `▸ N · name`; the active window's glyph is highlighted.
3. Cursor onto a window row, press Enter → tmux switches to that window.
4. Press `w` on a no-git-root session, type a name, Enter → new window appears within ~2s; the switcher lists it.
5. Press `+` on a no-git-root row → status reads `no git root — press w to add a window`.
6. Press `tab` → per-window recap `※` lines appear under windows running Claude; press `tab` again → they hide.
7. Confirm a single-window session renders identically to before (no window rows).
8. `?` overlay shows `w new window`.

---

## Self-Review notes

- **Spec coverage:** scope (>1 window) → Task 3 Step 4; separate `w`/`+` verbs → Tasks 5 & 6; session-row-plus-sub-rows layout → Task 3; per-window inline status + tab-gated recap → Tasks 2 & 3; inline name prompt → Task 5; deferred window-close → intentionally out of scope; per-window AI data reuse → Task 1; attach-target widening → Task 4; help/guidance → Task 6. All spec sections mapped.
- **Key-binding correction:** spec loosely said `p` for provision; the actual binding is `+` (`p` is unused, `P` is preview popup). Plan uses `+` for provision and binds the new `w`. This is a deliberate deviation from the spec's shorthand, consistent with the spec's *intent*.
- **Type consistency:** `windowInfo` fields identical across Tasks 2–5; `aggregateAIStatesByWindow` signature identical in Tasks 1 & 2; `newWindowArgs`/`newWindowInSession` consistent in Task 5.
- **Known limitation carried from spec:** shared-cwd windows both running Claude resolve to most-recently-updated recap (documented, accepted for v1).
