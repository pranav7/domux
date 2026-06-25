# domux-start plugin skill Implementation Plan

> **For agentic workers:** Implement this plan task-by-task. Each task ends with a build/test/commit cycle. Steps use checkbox (`- [ ]`) syntax for tracking. Do the tasks in order; each is independently committable.

**Goal:** Ship the task-kickoff workflow as a `domux-start` marketplace plugin skill, retire the embedded `/start-task` command, and add a `/cr` alias for `/codex-review`.

**Architecture:** Three independent changes in one repo. (A) New plugin dir `plugins/domux-start/` registered in `.claude-plugin/marketplace.json`, carrying the start-task workflow as `skills/domux-start/SKILL.md`. (B) Remove the `//go:embed` + install write + tests + README mention for `/start-task`. (C) Add `plugins/implement-pipeline/commands/cr.md` forwarding to the `codex-review` skill.

**Tech Stack:** Go 1.22 (`package main`, single-package layout), Claude Code plugin format (markdown SKILL.md / command files, JSON manifests).

## Global Constraints

- Go 1.22, single `package main`, no module subdirs. Build with `go build`, test with `go test ./...`, vet with `go vet ./...`.
- Restricted branches — never commit directly to: `main`, `master`, `workspace-*`. Work happens on `add-domux-start-skill` (already checked out).
- Atomic-write / backup conventions live in `install.go`; this plan only *removes* an install write, so no new write code.
- Plugin `plugin.json` shape mirrors the existing `plugins/domux-communicate/.claude-plugin/plugin.json`: keys `name`, `version`, `description`, `author.name`, `homepage`.
- The built-in `/review` (PR review) must stay reachable — the codex-review alias is `/cr`, NOT `/review`.
- Commit message trailer for every commit: `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## File Structure

- `plugins/domux-start/.claude-plugin/plugin.json` — NEW. Plugin manifest.
- `plugins/domux-start/skills/domux-start/SKILL.md` — NEW. The kickoff skill.
- `.claude-plugin/marketplace.json` — MODIFY. Add third plugin entry.
- `install.go` — MODIFY. Remove start-task embed/const/write + dead helper.
- `assets/claude/commands/start-task.md` — DELETE (and empty parent dirs).
- `install_test.go` — MODIFY. Remove the two start-task tests.
- `README.md` — MODIFY. Replace the `/start-task` paragraph.
- `plugins/implement-pipeline/commands/cr.md` — NEW. `/cr` alias command.

---

## Task 1: Create the `domux-start` plugin

**Files:**
- Create: `plugins/domux-start/.claude-plugin/plugin.json`
- Create: `plugins/domux-start/skills/domux-start/SKILL.md`
- Modify: `.claude-plugin/marketplace.json`

**Interfaces:**
- Produces: a plugin named `domux-start` whose source is `./plugins/domux-start`, discoverable in the domux marketplace; a skill `domux-start` that auto-triggers on task kickoff.
- Consumes: nothing from other tasks.

- [ ] **Step 1: Write the plugin manifest**

Create `plugins/domux-start/.claude-plugin/plugin.json`:

```json
{
  "name": "domux-start",
  "version": "0.1.0",
  "description": "Kick off a new task in a tmux+domux worktree: resolve the workspace, branch off fresh main, label the session, then start coding.",
  "author": { "name": "pranav7" },
  "homepage": "https://github.com/pranav7/domux"
}
```

- [ ] **Step 2: Write the skill**

Create `plugins/domux-start/skills/domux-start/SKILL.md` with this exact content (note: `$ARGUMENTS` from the old command is replaced with prose, since skills do not substitute placeholders):

````markdown
---
name: domux-start
description: Use when starting or kicking off a new task in a tmux+domux workbench — sets up the workspace before any code: resolves the git worktree root, branches off fresh origin/main, labels the domux session so the human can find it in the switcher, then begins the task. Invoke at the start of task-shaped requests in a domux/tmux session, whether triggered as /domux-start <task> or auto-detected.
---

# /domux-start — kick off a task in a tmux+domux workbench

You're being kicked off in a tmux+domux workbench. The task to start is
whatever you were asked to do — the argument passed to `/domux-start`, or the
request you're currently acting on. Set up the workspace before touching code.

## What domux is

`domux` pins each tmux session to a git **worktree root** and tracks per-session
state (label, AI status, focused TODO). Worktrees are independent checkouts of
the same repo at different paths — typically one per in-flight task — so each
session is meant to be a fresh, short-lived branch off `origin/main`.

- Session is pinned to the worktree root, so `cd`ing into subdirs is fine.
- TODO list lives at the path printed by `domux --path`.
- Session label shows in the switcher; set it so the human can find you.

**Restricted branches — never commit directly to:** `main`, `master`, `workspace-*`.

## Setup steps (do these before anything else)

1. **Resolve the workspace.**
   - `git rev-parse --show-toplevel` → worktree root.
   - `git worktree list` → check whether this checkout is the primary or a worktree.
   - `git status --porcelain` → check for uncommitted changes.

2. **Refresh main.** `git fetch origin main`.

3. **Branch off fresh main.** Pick a kebab-case branch name from the task
   (e.g. `fix-login-redirect`, `add-export-csv` — short, 2–5 words).
   - **If this is a worktree AND it's clean:** `git checkout main && git reset --hard origin/main && git checkout -b <branch>`. Worktrees are meant to be wiped between tasks.
   - **If this is the primary checkout OR there are uncommitted changes:** do NOT reset. Just `git checkout -b <branch> origin/main` (or stop and ask the user how to handle the dirty state).

4. **Label the session** so it shows up in the domux switcher:
   `domux label set "<2–4 word task title>"`. Current tmux session is auto-detected.

5. **Acknowledge in one line**, then start the task.

## domux quick reference

| Command | What it does |
|---|---|
| `domux --path` | Print the pinned TODO file for this session |
| `domux label set "..."` | Name the current session (shown in switcher) |
| `domux label clear` | Clear the session name |
| `domux sessions` | Open the session switcher TUI |
| `domux todo` | Open the per-worktree TODO TUI |
| `domux --status` | Top active task (for status bars) |

Sessions, labels, and AI state are all auto-managed by hooks — you only need
to set the label and pick the branch. Everything else just works.
````

- [ ] **Step 3: Register the plugin in the marketplace**

In `.claude-plugin/marketplace.json`, add a third object to the `plugins` array (after the `domux-communicate` entry). The array becomes:

```json
  "plugins": [
    {
      "name": "implement-pipeline",
      "source": "./plugins/implement-pipeline",
      "description": "Hands-off /implement pipeline: implement → simplify → lint → verify → review → PR, reusing Claude's built-in skills."
    },
    {
      "name": "domux-communicate",
      "source": "./plugins/domux-communicate",
      "description": "Message the Claude agent in another domux worktree: domux send/read/peek + the /domux-communicate skill."
    },
    {
      "name": "domux-start",
      "source": "./plugins/domux-start",
      "description": "Kick off a task in a tmux+domux worktree: branch off fresh main, label the session, then start — the /domux-start skill."
    }
  ]
```

- [ ] **Step 4: Validate the JSON and skill frontmatter**

Run: `python3 -m json.tool .claude-plugin/marketplace.json > /dev/null && python3 -m json.tool plugins/domux-start/.claude-plugin/plugin.json > /dev/null && echo OK`
Expected: `OK` (both files are valid JSON).

Run: `head -4 plugins/domux-start/skills/domux-start/SKILL.md`
Expected: shows the `---` / `name: domux-start` / `description:` / `---` frontmatter block.

- [ ] **Step 5: Commit**

```bash
git add plugins/domux-start .claude-plugin/marketplace.json
git commit -m "feat: add domux-start plugin skill for task kickoff

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Retire the embedded `/start-task` command

**Files:**
- Modify: `install.go` (remove embed line 17–18, const line 20, cmdPath line 112, preview line 117, write call line 136, dead helper `writeClaudeCommand` lines 142–162)
- Delete: `assets/claude/commands/start-task.md` (and empty parent dirs)
- Modify: `install_test.go` (remove `TestInstallClaudeWritesStartTaskCommand` lines 87–106 and `TestInstallClaudeStartTaskIsIdempotent` lines 108–131)
- Modify: `README.md` (lines 302–303)

**Interfaces:**
- Consumes: nothing.
- Produces: `installClaude` no longer writes `~/.claude/commands/start-task.md`; it still patches settings and installs the marketplace. No exported symbol changes.

- [ ] **Step 1: Remove the embed directive, var, and const in `install.go`**

Delete these three lines near the top of `install.go` (currently lines 17–20):

```go
//go:embed assets/claude/commands/start-task.md
var claudeStartTaskCommand string

const claudeStartTaskFile = "start-task.md"
```

After removal, check whether the `_ "embed"` import (line 4) is still needed. Run `grep -n 'go:embed' install.go *.go`. If no other `//go:embed` remains anywhere in the package, remove the `_ "embed"` import line too; otherwise leave it.

- [ ] **Step 2: Remove the cmdPath line and the write call in `installClaude`**

In `installClaude`, delete the `cmdPath` assignment (currently line 112):

```go
	cmdPath := filepath.Join(homeDir, ".claude", "commands", claudeStartTaskFile)
```

Delete the preview line (currently line 117) inside the `if !*apply` block:

```go
		fmt.Printf("\nWould write %s (the /start-task slash command).\n", cmdPath)
```

Delete the write call (currently lines 136–138) so the tail of `installClaude` goes from:

```go
	fmt.Printf("patched %s\n", path)
	if err := writeClaudeCommand(cmdPath, claudeStartTaskCommand); err != nil {
		return err
	}
	return runPluginInstall(marketplaceSource)
}
```

to:

```go
	fmt.Printf("patched %s\n", path)
	return runPluginInstall(marketplaceSource)
}
```

- [ ] **Step 3: Remove the now-dead `writeClaudeCommand` helper**

Delete the entire `writeClaudeCommand` function (currently lines 142–162):

```go
func writeClaudeCommand(path, content string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", filepath.Dir(path), err)
	}
	existing, err := os.ReadFile(path)
	if err == nil && string(existing) == content {
		return nil
	}
	if err == nil {
		if err := backupIfExists(path); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("cannot read %s: %w", path, err)
	}
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		return fmt.Errorf("cannot write %s: %w", path, err)
	}
	fmt.Printf("wrote %s\n", path)
	return nil
}
```

(`homeDir` is still used by the `path` assignment earlier in `installClaude`, so leave that. `backupIfExists`, `os`, `fmt`, `filepath` all remain used elsewhere in the file — do not touch their imports.)

- [ ] **Step 4: Delete the asset file and empty dirs**

Run:

```bash
git rm assets/claude/commands/start-task.md
rmdir assets/claude/commands assets/claude 2>/dev/null; true
```

(`rmdir` removes the now-empty dirs; the `2>/dev/null; true` keeps it quiet if `assets/claude` still holds other files — check with `ls assets/claude 2>/dev/null` and leave it if non-empty.)

- [ ] **Step 5: Remove the two start-task tests in `install_test.go`**

Delete `TestInstallClaudeWritesStartTaskCommand` (currently lines 87–106) and `TestInstallClaudeStartTaskIsIdempotent` (currently lines 108–131) in their entirety. These are the only two tests that reference `start-task.md`.

(The `filepath` and `strings` imports stay used by other tests in the file — `filepath` at lines 32/49/61/170/338, `strings` at 205/208/316/319/334 — so leave the import block alone.)

- [ ] **Step 6: Update the README**

In `README.md`, replace the two-line paragraph (currently lines 302–303):

```
The Claude install also writes a `/start-task` command that tells Claude how to
use domux, tmux sessions, and git worktrees before it starts coding.
```

with:

```
The task-kickoff workflow (set up the worktree, branch off fresh `main`, label
the session, then start coding) ships as the `domux-start` Claude Code plugin,
installed from the domux marketplace alongside the others. Invoke it with
`/domux-start <task>`.
```

- [ ] **Step 7: Build and verify nothing references the removed symbols**

Run: `go build ./... && go vet ./...`
Expected: no output, exit 0. (`go vet` flags any leftover reference to `claudeStartTaskCommand`, `claudeStartTaskFile`, `cmdPath`, or `writeClaudeCommand`.)

Run: `grep -rn "start-task\|claudeStartTask\|writeClaudeCommand" --include='*.go' .`
Expected: no matches.

- [ ] **Step 8: Run the install tests**

Run: `go test -run TestInstallClaude ./...`
Expected: PASS (the remaining install tests pass; the two removed tests are gone).

- [ ] **Step 9: Manual preview sanity check**

Run: `HOME=$(mktemp -d) go run . install claude`
Expected: prints the settings patch + the plugin install plan, and **no** "Would write … /start-task" line.

- [ ] **Step 10: Commit**

```bash
git add install.go install_test.go README.md assets
git commit -m "refactor: retire embedded /start-task command in favor of domux-start plugin

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Add the `/cr` alias for `/codex-review`

**Files:**
- Create: `plugins/implement-pipeline/commands/cr.md`

**Interfaces:**
- Consumes: the existing `codex-review` skill in `plugins/implement-pipeline/skills/codex-review/SKILL.md`.
- Produces: a `/cr` command that forwards to that skill. Does not touch the built-in `/review`.

- [ ] **Step 1: Write the command file**

Create `plugins/implement-pipeline/commands/cr.md` with this exact content:

```markdown
---
description: Short alias for /codex-review — external second-opinion review of the current changes via Codex (GPT-5.5 on Azure).
argument-hint: "[optional extra context for the reviewer]"
---

Run an external code review of the current changes using the `codex-review`
skill. Invoke the **codex-review** skill now and follow it exactly. Pass along
any arguments the user provided as extra context for the reviewer: $ARGUMENTS
```

- [ ] **Step 2: Validate frontmatter and confirm `/review` is untouched**

Run: `head -4 plugins/implement-pipeline/commands/cr.md`
Expected: shows the `---` / `description:` / `argument-hint:` / `---` frontmatter.

Run: `find plugins -name 'review.md'`
Expected: no output (we did NOT create a `review.md`; the built-in `/review` is untouched).

- [ ] **Step 3: Commit**

```bash
git add plugins/implement-pipeline/commands/cr.md
git commit -m "feat: add /cr alias for codex-review skill

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Part A (new plugin) → Task 1 (plugin.json, SKILL.md, marketplace entry). ✓
- Part B (retire /start-task) → Task 2 (install.go, asset delete, two tests removed, README). ✓ All four spec edit sites covered.
- Part C (/cr alias) → Task 3. ✓ Built-in `/review` left untouched per the decision (alias is `/cr`).
- Spec's `$ARGUMENTS` adaptation note → handled in Task 1 Step 2 (prose replaces the placeholder). ✓
- Verification section of spec (`go build`/`go test`/`go vet`/preview/plugin validity) → Task 2 Steps 7–9 and Task 1 Step 4. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases"/"similar to". All file contents shown in full. The `$ARGUMENTS` token inside the `cr.md` content (Task 3) is intentional — it is the command-file substitution variable, not a plan placeholder.

**Type/name consistency:** Symbol names removed in Task 2 (`claudeStartTaskCommand`, `claudeStartTaskFile`, `writeClaudeCommand`, `cmdPath`) match `install.go` exactly. Plugin name `domux-start` and source `./plugins/domux-start` are consistent across plugin.json, marketplace.json, and SKILL.md `name`. ✓
