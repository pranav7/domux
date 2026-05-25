# Implement Workflow Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `/implement` Claude Code plugin (and individually invokable sub-commands) from the domux repository that runs a hands-off implementation pipeline: implement → simplify → lint → browser test → UX review → azcodex external review → PR.

**Architecture:** New top-level `.claude-plugin/marketplace.json` declares the domux marketplace. Plugin lives at `plugins/implement-pipeline/` with one orchestrator skill, three specialist skills (browser-test, ux-review, azcodex-review), and four slash commands. Cross-marketplace dependencies on official `code-simplifier`, `frontend-design`, `typescript-lsp`, `pyright-lsp`. `domux install claude --apply` shells out to `claude plugin marketplace add` + `claude plugin install` after patching `~/.claude/settings.json`. No new Go embeds — Claude Code reads plugin files directly.

**Tech Stack:** Go (install.go integration + tests), Markdown (skills + commands), JSON (manifests), Bash (lint/azcodex inline). External: `claude` CLI, `azcodex` (= `codex --profile azure`), official Anthropic plugins.

**Spec:** `docs/superpowers/specs/2026-05-25-implement-workflow-design.md`

---

## File Structure

**New files (domux repo):**

```
domux/
  .claude-plugin/
    marketplace.json                        ← marketplace catalog
  plugins/implement-pipeline/
    .claude-plugin/
      plugin.json                           ← plugin manifest
    commands/
      implement.md
      browser-test.md
      ux-review.md
      azcodex-review.md
    skills/
      implement-workflow/
        SKILL.md                            ← orchestrator
        simplify-prompt.md
        lint-prompt.md
        browser-test-prompt.md
        ux-review-prompt.md
        azcodex-prompt.md
      browser-test/SKILL.md
      ux-review/SKILL.md
      azcodex-review/SKILL.md
  docs/implement-pipeline.md                ← user-facing usage doc
```

**Modified files:**

- `install.go` — add plugin marketplace registration + install at end of `installClaude`
- `install_test.go` — new tests for plugin install behavior + manifest validity
- `.gitignore` — add `.claude-pipeline/`
- `README.md` — short mention of `/implement` + link to docs/implement-pipeline.md
- `Makefile` — add `smoke-install-claude` target

**File responsibilities:**

- `marketplace.json` — declares one plugin entry + `allowCrossMarketplaceDependenciesOn`
- `plugin.json` — declares dependencies + version
- `implement-workflow/SKILL.md` — single source of truth for the pipeline orchestration. Reads inputs, sequences gates, manages state file, handles fail-loops. The other prompts under `implement-workflow/` are templates the orchestrator inlines when dispatching subagents.
- `browser-test/SKILL.md`, `ux-review/SKILL.md`, `azcodex-review/SKILL.md` — standalone, individually invokable. Same gate contract (`VERDICT:` line).
- `commands/*.md` — thin shims that invoke the corresponding skill.

---

## Task 1: Plugin scaffolding — marketplace.json + plugin.json

**Files:**
- Create: `.claude-plugin/marketplace.json`
- Create: `plugins/implement-pipeline/.claude-plugin/plugin.json`

- [ ] **Step 1: Write `marketplace.json`**

`.claude-plugin/marketplace.json`:

```json
{
  "name": "domux",
  "owner": { "name": "pranav7", "url": "https://github.com/pranav7" },
  "allowCrossMarketplaceDependenciesOn": ["claude-plugins-official"],
  "plugins": [
    {
      "name": "implement-pipeline",
      "source": "./plugins/implement-pipeline",
      "description": "Hands-off implementation pipeline: simplify + lint + browser test + UX review + external code review."
    }
  ]
}
```

- [ ] **Step 2: Write `plugin.json`**

`plugins/implement-pipeline/.claude-plugin/plugin.json`:

```json
{
  "name": "implement-pipeline",
  "version": "0.1.0",
  "description": "Hands-off implementation pipeline orchestrating simplify, lint, browser test, UX review, and azcodex external review.",
  "author": { "name": "pranav7" },
  "homepage": "https://github.com/pranav7/domux",
  "dependencies": [
    { "name": "code-simplifier",  "marketplace": "claude-plugins-official" },
    { "name": "frontend-design",  "marketplace": "claude-plugins-official" },
    { "name": "typescript-lsp",   "marketplace": "claude-plugins-official" },
    { "name": "pyright-lsp",      "marketplace": "claude-plugins-official" }
  ]
}
```

- [ ] **Step 3: Verify JSON parses**

Run: `python3 -c "import json; json.load(open('.claude-plugin/marketplace.json')); json.load(open('plugins/implement-pipeline/.claude-plugin/plugin.json')); print('OK')"`
Expected: `OK`

- [ ] **Step 4: Commit**

```bash
git add .claude-plugin/marketplace.json plugins/implement-pipeline/.claude-plugin/plugin.json
git commit -m "plugin: scaffold implement-pipeline marketplace + manifest"
```

---

## Task 2: Gitignore `.claude-pipeline/` runtime artifacts

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Add `.claude-pipeline/` to `.gitignore`**

Append a single line so it sits with the other generated dirs:

```
.claude-pipeline/
```

After change, `.gitignore` should read:

```
domux
dist/
.DS_Store
.worktrees/
.claude-pipeline/
```

- [ ] **Step 2: Verify gitignore matches**

Run: `git check-ignore .claude-pipeline/state.json`
Expected: `.claude-pipeline/state.json` (echoed back — means it's ignored)

- [ ] **Step 3: Commit**

```bash
git add .gitignore
git commit -m "gitignore: ignore .claude-pipeline runtime artifacts"
```

---

## Task 3: Orchestrator skill — `implement-workflow/SKILL.md`

**Files:**
- Create: `plugins/implement-pipeline/skills/implement-workflow/SKILL.md`

- [ ] **Step 1: Write the orchestrator skill**

`plugins/implement-pipeline/skills/implement-workflow/SKILL.md`:

````markdown
---
name: implement-workflow
description: Hands-off implementation pipeline. Picks up a spec/plan/issue/task, runs implement → simplify → lint → browser test → UX review → azcodex review → PR, with babysitting. Use when the user runs /implement or asks to "implement X end to end".
---

# Implementation Pipeline Orchestrator

You sequence eight gated stages over the current branch. Each gate emits `VERDICT: PASS|FAIL|ESCALATE`. You parse it, persist state, and decide what to do next.

## Inputs

Arguments form (from `/implement`):

| Arg                  | Behavior                                                                                     |
|----------------------|----------------------------------------------------------------------------------------------|
| (none)               | Resume in-progress run, else newest unfinished plan in `docs/superpowers/plans/`, else ask. |
| `<path>`             | Classify by frontmatter+path: spec → invoke `superpowers:writing-plans` → execute; plan → execute. |
| `"free text"`        | Small task; skip plan-writing; informal one-task plan in-line.                              |
| `LIN-<id>`           | Defer to `/fix-linear-issue`; then run the pipeline.                                        |
| `#<n>`               | `gh issue view <n>` → treat body as informal task.                                          |
| `--resume`           | Read `.claude-pipeline/state.json`; skip completed stages.                                  |
| `--from <stage>`     | Re-run from a specific gate.                                                                |
| `--skip <stage,…>`   | Skip specific gates.                                                                        |
| `--no-pr`            | Stop after stage 7.                                                                         |

## Preconditions

Before starting any new run (i.e. not `--resume`):

1. `git status --porcelain` must be empty. If not, refuse: `pipeline refuses to start with dirty tree. Commit or stash first, or pass --resume.`
2. Detect base branch: `git symbolic-ref refs/remotes/origin/HEAD | sed 's@^refs/remotes/origin/@@'` → default `main`.
3. Compute diff scope once: `BASE=<base>; CHANGED=$(git diff --name-only "$BASE"...HEAD)`. Hold for skip rules.
4. Ensure `.claude-pipeline/` exists (mkdir -p). Confirm `.claude-pipeline/` is in `.gitignore`; append if missing.

## Skip rules (run once at start)

- Python lint: any `*.py` in `$CHANGED`.
- JS/TS lint: any `*.ts`, `*.tsx`, `*.js`, `*.jsx` in `$CHANGED`.
- UI (browser test + UX review): any `*.tsx`, `*.jsx`, `*.html`, `*.css` in `$CHANGED`, OR path under `pages/`, `app/`, `routes/`, `components/`.

## Stages

Always run unless a skip rule fires or `--skip` is set.

| # | Stage         | How                                                                                      |
|---|---------------|------------------------------------------------------------------------------------------|
| 1 | Implement     | Invoke `superpowers:subagent-driven-development` with the resolved plan/task.            |
| 2 | Simplify      | Dispatch the `code-simplifier:code-simplifier` agent via the `Agent` tool on the diff.   |
| 3 | Lint auto-fix | Inline Bash one-liner. See "Lint detail" below.                                          |
| 4 | Browser test  | Invoke the `implement-pipeline:browser-test` skill.                                      |
| 5 | UX + scope    | Invoke the `implement-pipeline:ux-review` skill.                                         |
| 6 | azcodex       | Invoke the `implement-pipeline:azcodex-review` skill.                                    |
| 7 | Address       | Fresh `general-purpose` subagent with findings + diff. See "Address findings" below.    |
| 8 | PR + babysit  | Invoke `/commit-push-pr` then `/loop 5m /babysit`. Skip if `--no-pr`.                    |

### Stage 2 — Simplify

Read `simplify-prompt.md` (sibling file) for the dispatch prompt template. Substitute `{{BASE}}` then dispatch the `code-simplifier:code-simplifier` agent via the `Agent` tool. Treat any returned text without explicit errors as PASS.

### Stage 3 — Lint auto-fix

Read `lint-prompt.md` for the full detection block. The Bash to run:

```bash
# Python
if [ -f pyproject.toml ] && echo "$CHANGED" | grep -q '\.py$'; then
  if command -v ruff >/dev/null; then
    ruff check --fix . && ruff format .
  fi
fi
# JS/TS
if echo "$CHANGED" | grep -qE '\.(ts|tsx|js|jsx)$'; then
  if [ -f biome.json ] || [ -f biome.jsonc ]; then
    npx --no-install biome check --apply .
  elif [ -f package.json ]; then
    files=$(echo "$CHANGED" | grep -E '\.(ts|tsx|js|jsx)$')
    npx --no-install eslint --fix $files || true
    npx --no-install prettier --write $files || true
  fi
fi
```

After running: `git diff --quiet` — if non-zero, commit `chore: lint auto-fix`. Then re-run the checker (without `--fix`) to confirm clean:

```bash
ruff check . && (biome check . || eslint <files>)
```

Non-zero from the post-check → `VERDICT: FAIL` with stderr.

### Stages 4–6 — Specialist skills

Invoke the named skill via the `Skill` tool (`implement-pipeline:browser-test`, etc.). Pass `{{BASE}}` + spec/plan path + previous artifacts dir as args.

### Stage 7 — Address findings

If azcodex returns any `[CRITICAL]` or `[BLOCKING]` items:

1. Read `.claude-pipeline/azcodex.md` + current diff.
2. Dispatch a `general-purpose` subagent via the `Agent` tool with prompt template `azcodex-prompt.md`.
3. After it commits, re-run stages 3, 4, 6.
4. Max 2 iterations; on 3rd entry → `VERDICT: ESCALATE: max iterations exceeded`.

### Stage 8 — PR + babysit

`/commit-push-pr` (the user's existing dotfiles command) opens the PR. Then `/loop 5m /babysit` to watch CI and address reviews. Skip both if `--no-pr`.

## State management

State file: `.claude-pipeline/state.json` (gitignored).

```json
{
  "run_id": "YYYY-MM-DD-HHMM",
  "input": "<path-or-text>",
  "input_kind": "spec|plan|free-text|linear|gh-issue",
  "base": "main",
  "stages": {
    "implement": {"status": "pending|in_progress|completed|failed", "commit": "<sha>", "attempts": 1},
    "simplify":  {"status": "...", "commit": "<sha>", "attempts": 1},
    "lint":      {"status": "...", "commit": "<sha>", "attempts": 1},
    "browser":   {"status": "...", "attempts": 1},
    "ux":        {"status": "...", "attempts": 1},
    "azcodex":   {"status": "...", "attempts": 1},
    "address":   {"status": "...", "attempts": 0},
    "pr":        {"status": "..."}
  },
  "started_at": "<ISO-8601>"
}
```

- Write the file at run start. Update one stage at a time, atomically (write to `state.json.tmp`, rename).
- `--resume` reads it and starts at the first non-`completed` stage.
- Standalone skills (e.g. `/browser-test` alone) also update their stage in the same file so a later `--resume` picks up correctly.

## Gate contract

Every gate (specialist skill, lint, azcodex) ends its output with a single line:

```
VERDICT: PASS
VERDICT: FAIL: <one-line reason>
VERDICT: ESCALATE: <one-line reason>
```

Parse the last `VERDICT:` line in the gate's output. On FAIL: stop and report to user. On ESCALATE: stop, report, do not retry. On PASS: continue.

## Artifacts

`.claude-pipeline/` layout:

```
.claude-pipeline/
  state.json
  azcodex.md
  artifacts/browser/{*.png,*.gif,console.log}
  findings/ux.md
```

## Resolving input

```
input_kind = classify(input):
  empty                → resume if state.json exists; else newest unfinished plan in docs/superpowers/plans/; else ask user.
  starts with LIN-     → linear
  starts with #        → gh-issue
  ends with .md
    + path matches /specs/  → spec
    + path matches /plans/  → plan
    + else                  → frontmatter status: "spec" or "plan"
  else                  → free-text
```

- spec → invoke `superpowers:writing-plans` first to produce a plan, then run stage 1 against the plan.
- plan → run stage 1 directly with the plan.
- free-text → write an informal 1-task plan in `.claude-pipeline/inline-plan.md` (NOT in docs/, since it's ephemeral) and run stage 1 against it.
- linear → invoke `/fix-linear-issue LIN-<id>` to land the issue context, then continue with the diff that produced.
- gh-issue → `gh issue view <n> --json title,body --jq '.title + "\n\n" + .body'` → treat as free-text.

## Final report

After stage 8 (or --no-pr early-stop), print a one-block summary:

```
implement-pipeline run <run_id>
  base:        <base>
  stages:      implement✓ simplify✓ lint✓ browser✓ ux✓ azcodex✓ address(0) pr✓
  PR:          <url> (if any)
  artifacts:   .claude-pipeline/
```
````

- [ ] **Step 2: Verify the file parses as plain markdown with valid frontmatter**

Run: `python3 -c "
import re, pathlib
p = pathlib.Path('plugins/implement-pipeline/skills/implement-workflow/SKILL.md')
s = p.read_text()
assert s.startswith('---'), 'missing frontmatter'
m = re.match(r'---\n(.*?)\n---\n', s, re.DOTALL)
assert m, 'malformed frontmatter'
fm = m.group(1)
assert 'name: implement-workflow' in fm
assert 'description:' in fm
print('OK')
"`
Expected: `OK`

- [ ] **Step 3: Commit**

```bash
git add plugins/implement-pipeline/skills/implement-workflow/SKILL.md
git commit -m "plugin: orchestrator SKILL.md for implement-workflow"
```

---

## Task 4: Orchestrator prompt templates

**Files:**
- Create: `plugins/implement-pipeline/skills/implement-workflow/simplify-prompt.md`
- Create: `plugins/implement-pipeline/skills/implement-workflow/lint-prompt.md`
- Create: `plugins/implement-pipeline/skills/implement-workflow/browser-test-prompt.md`
- Create: `plugins/implement-pipeline/skills/implement-workflow/ux-review-prompt.md`
- Create: `plugins/implement-pipeline/skills/implement-workflow/azcodex-prompt.md`

- [ ] **Step 1: Write `simplify-prompt.md`**

```markdown
# Simplify prompt — dispatched to code-simplifier:code-simplifier

Simplify the code changed in the current diff (vs `{{BASE}}`). Focus on:

- Reducing unnecessary complexity and nesting.
- Eliminating redundant code, abstractions, dead code paths.
- Improving naming clarity.

**Preserve all functionality exactly.** Do not change behavior. Do not introduce features.

After simplifying, run any project test suite that's wired up (`go test ./...`, `pytest`, `npm test`, etc.) to confirm nothing broke. If tests fail, revert the simplification.

Commit your changes with: `refactor: simplify recently changed code`

Output a one-line summary then:

VERDICT: PASS
```

- [ ] **Step 2: Write `lint-prompt.md`**

```markdown
# Lint detail (orchestrator reference)

This is reference for the orchestrator, not a dispatched prompt.

Detection order:

1. Python (`.py` in $CHANGED + pyproject.toml exists):
   - If `[tool.ruff]` in pyproject.toml → `ruff check --fix . && ruff format .`
   - Else if `[tool.black]` in pyproject.toml → `ruff check --fix . && black .` (ruff still does the lint fixes)
   - Else skip Python lint.

2. JS/TS (`.ts|.tsx|.js|.jsx` in $CHANGED):
   - If `biome.json` or `biome.jsonc` → `npx --no-install biome check --apply .`
   - Else if `package.json` → `npx --no-install eslint --fix <files> && npx --no-install prettier --write <files>`
   - Else skip JS/TS lint.

If anything was auto-fixed, commit `chore: lint auto-fix`. Re-run the checker (no `--fix`) and treat non-zero as FAIL.

LSP plugins (typescript-lsp, pyright-lsp) report residual issues to Claude in-context — no separate report step needed.
```

- [ ] **Step 3: Write `browser-test-prompt.md`**

```markdown
# Browser-test args (passed to implement-pipeline:browser-test skill)

- BASE: {{BASE}}
- SPEC_PATH: {{SPEC_PATH}} (may be empty)
- PLAN_PATH: {{PLAN_PATH}} (may be empty)
- CHANGED_UI_FILES: {{CHANGED_UI_FILES}}
- ARTIFACTS_DIR: .claude-pipeline/artifacts/browser/
```

- [ ] **Step 4: Write `ux-review-prompt.md`**

```markdown
# UX-review args (passed to implement-pipeline:ux-review skill)

- BASE: {{BASE}}
- SPEC_PATH: {{SPEC_PATH}}
- PLAN_PATH: {{PLAN_PATH}}
- BROWSER_ARTIFACTS: .claude-pipeline/artifacts/browser/
- FINDINGS_OUT: .claude-pipeline/findings/ux.md
```

- [ ] **Step 5: Write `azcodex-prompt.md`**

```markdown
# Address-findings prompt — dispatched to general-purpose subagent

The external reviewer (azcodex) flagged these findings in `.claude-pipeline/azcodex.md`:

{{FINDINGS_BODY}}

Current diff is against `{{BASE}}`. For each `[CRITICAL]` and `[BLOCKING]` finding:

1. Locate the offending code.
2. Apply the minimal fix that addresses the finding without expanding scope.
3. Add or update a test if behavior changed.
4. Re-run the project test suite.

`[NIT]` findings: address only if trivial (< 5 minutes each). Skip otherwise — leave a note in the commit body.

Commit fixes incrementally with clear messages (`fix: address azcodex finding — <one-line>`).

When done, output:

VERDICT: PASS
```

- [ ] **Step 6: Commit**

```bash
git add plugins/implement-pipeline/skills/implement-workflow/*.md
git commit -m "plugin: orchestrator prompt templates"
```

---

## Task 5: Browser-test specialist skill

**Files:**
- Create: `plugins/implement-pipeline/skills/browser-test/SKILL.md`

- [ ] **Step 1: Write the browser-test skill**

```markdown
---
name: browser-test
description: Spin up a local dev server, exercise UI routes touched by the current diff in a real browser, capture screenshots/console/network logs. Use when the user runs /browser-test or as stage 4 of /implement.
---

# Browser Test

Exercise the primary flows on each UI route changed by the current diff (vs base branch). Capture artifacts to `.claude-pipeline/artifacts/browser/`.

## Inputs

If args provided, parse `BASE`, `SPEC_PATH`, `PLAN_PATH`, `CHANGED_UI_FILES`, `ARTIFACTS_DIR`. Otherwise:

- BASE: `git symbolic-ref refs/remotes/origin/HEAD | sed 's@^refs/remotes/origin/@@'` (default `main`).
- CHANGED_UI_FILES: `git diff --name-only "$BASE"...HEAD | grep -E '\.(tsx|jsx|html|css)$|^(pages|app|routes|components)/'`.
- ARTIFACTS_DIR: `.claude-pipeline/artifacts/browser/`.

## Procedure

1. **Infer routes.** From changed files, map to URL paths. (e.g. `app/settings/page.tsx` → `/settings`; `pages/users/[id].tsx` → `/users/123`.) If unclear, ask the user.

2. **Ensure dev server is running.**
   - `lsof -nP -iTCP -sTCP:LISTEN | grep -E ':(3000|5173|8080|4173)\s'` to check common ports.
   - If nothing's listening, look at `package.json` scripts for `dev|start|serve` and start it in background: `npm run dev > .claude-pipeline/dev-server.log 2>&1 &` (or yarn/pnpm equivalent based on lockfile). Wait up to 30s for port to open.

3. **Login wall.** Try `.env.test` creds if present (`TEST_USER`, `TEST_PASS`). If login fails or no creds:
   - Slack-DM `<@U0ADJBVPGUC>` in `#all_` (channel `C0ACZAG5QVD`) with:
     ```
     <@U0ADJBVPGUC> browser test waiting on login at <localhost-url>. React with ✅ when done.
     ```
   - Spawn a background subagent via `Agent` tool that polls thread + channel history with exponential backoff (1m, 3m, 9m). Resume when ✅ is seen.
   - If 13 minutes pass with no ✅: `VERDICT: ESCALATE: login-wall timeout`.

4. **Exercise each route.** Use the `claude-in-chrome` MCP tools (already on Pranav's system):
   - Navigate (`mcp__claude-in-chrome__navigate`).
   - Capture screenshot via `mcp__claude-in-chrome__gif_creator` (start recording before interaction, end after).
   - Read console messages (`mcp__claude-in-chrome__read_console_messages`); look for errors/warnings introduced by the change.
   - For each interactive element on the page that the diff touches, click/fill it; capture before/after screenshots.
   - Save artifacts as `{route-slug}.png`, `{route-slug}.gif`, `{route-slug}.console.log` in `$ARTIFACTS_DIR`.

5. **Evaluate.** A route passes if:
   - It loads with HTTP 200 (no 4xx/5xx).
   - No new console errors vs base.
   - Touched interactive elements respond (don't 404, don't throw uncaught exceptions).

## Output

End with the verdict line. List passed/failed routes inline.

```
Routes tested:
  /settings    PASS    artifacts/browser/settings.png
  /users/123   FAIL    console error: ReferenceError: foo

VERDICT: FAIL: 1 of 2 routes failed (see above)
```

PASS / FAIL / ESCALATE — pick exactly one for the trailing VERDICT line.
```

- [ ] **Step 2: Commit**

```bash
git add plugins/implement-pipeline/skills/browser-test/SKILL.md
git commit -m "plugin: browser-test specialist skill"
```

---

## Task 6: UX-review specialist skill

**Files:**
- Create: `plugins/implement-pipeline/skills/ux-review/SKILL.md`

- [ ] **Step 1: Write the ux-review skill**

```markdown
---
name: ux-review
description: Evaluate UI changes for design-system adherence, sizing/spacing/alignment, dead-end buttons, and scope discipline (every element traces to a spec requirement). Invokes frontend-design:frontend-design internally. Use when the user runs /ux-review or as stage 5 of /implement.
---

# UX + Scope Review

Two passes over the diff:

1. **Design pass** — invoke `frontend-design:frontend-design` skill for design-system / sizing / spacing / alignment evaluation.
2. **Scope pass** — verify every UI element in the diff traces to a spec/plan requirement. Flag dead-end buttons, unreachable routes, components that exist but are never rendered, features that exist but weren't asked for.

## Inputs

If args provided, parse `BASE`, `SPEC_PATH`, `PLAN_PATH`, `BROWSER_ARTIFACTS`, `FINDINGS_OUT`. Otherwise infer from current branch (same logic as browser-test skill).

## Design pass

Invoke `frontend-design:frontend-design` via the `Skill` tool with the diff + browser screenshots from `$BROWSER_ARTIFACTS`. Capture its findings.

## Scope pass

Read the spec (`$SPEC_PATH`) and/or plan (`$PLAN_PATH`) if present.

For each new/modified UI element in the diff:
- **New button/link.** Where does it lead? Does the destination handle the click? If the destination doesn't exist or is a stub → **dead-end**.
- **New route.** Is it linked from somewhere? Anywhere?
- **New component.** Is it rendered? By what?
- **New feature flag / setting / form field.** Does the spec mention it? If not → **out-of-scope**.

If spec/plan is empty (free-text run), use the user's original prompt as the source of truth instead.

## Output

Write findings to `$FINDINGS_OUT` (default `.claude-pipeline/findings/ux.md`).

```markdown
# UX + Scope Review — <date>

## Design (from frontend-design)
{{frontend-design output}}

## Scope
{{list}}

## Summary
- P0 (blocker): N
- P1 (must-fix): N
- P2 (nice): N
```

Severity:
- **P0** — dead-end button, broken navigation, missing-from-spec critical-path feature.
- **P1** — design-system violation visible to user (wrong button size, bad spacing on primary CTA, color out of palette).
- **P2** — copy nits, minor alignment.

End with the verdict line:

- PASS if zero P0 and zero P1.
- FAIL otherwise — include count.

```
VERDICT: PASS
VERDICT: FAIL: 1 P0, 2 P1 (see .claude-pipeline/findings/ux.md)
```
```

- [ ] **Step 2: Commit**

```bash
git add plugins/implement-pipeline/skills/ux-review/SKILL.md
git commit -m "plugin: ux-review specialist skill"
```

---

## Task 7: Azcodex-review specialist skill

**Files:**
- Create: `plugins/implement-pipeline/skills/azcodex-review/SKILL.md`

- [ ] **Step 1: Write the azcodex-review skill**

```markdown
---
name: azcodex-review
description: External code review via azcodex (codex --profile azure). Reviews diff vs base for correctness, security, regressions, and spec compliance. Parses severity markers, emits VERDICT. Use when the user runs /azcodex-review or as stage 6 of /implement.
---

# Azcodex External Review

Run a non-interactive second-opinion review using `azcodex` (alias for `codex --profile azure`).

## Preconditions

- `azcodex` must be on PATH. If `command -v azcodex` fails: `VERDICT: ESCALATE: azcodex not installed`.
- Working tree must be clean (the gate sees committed changes only). If dirty: `VERDICT: ESCALATE: working tree dirty`.

## Inputs

- BASE: default `main`. Override via args.
- SPEC_PATH / PLAN_PATH: if provided, included in the prompt for compliance check.
- OUT: default `.claude-pipeline/azcodex.md`.

## Procedure

Build prompt:

```
Review the changes on the current branch vs `<BASE>` for:
- Correctness bugs (off-by-one, null derefs, wrong assumptions about callers).
- Security (auth bypass, injection, secrets in code, unsafe deserialization).
- Regressions (existing tests still cover changed code paths; new code paths have tests).
- Spec compliance (every change traces to a requirement in <SPEC_PATH or PLAN_PATH>, if attached).

Annotate each finding with one of: [CRITICAL], [BLOCKING], [NIT].
- [CRITICAL] = data loss, security, prod-breaking.
- [BLOCKING] = must-fix before merge.
- [NIT] = minor.
```

If SPEC_PATH or PLAN_PATH provided, append:

```
Spec/plan for compliance check:
<contents>
```

Invoke:

```bash
azcodex review --base "$BASE" "$PROMPT" > "$OUT"
```

Time out after 5 minutes (`timeout 300 azcodex review …`). On timeout → `VERDICT: ESCALATE: azcodex timed out`.

## Parsing

Count `[CRITICAL]` and `[BLOCKING]` occurrences:

```bash
CRIT=$(grep -c '\[CRITICAL\]' "$OUT" || true)
BLOCK=$(grep -c '\[BLOCKING\]' "$OUT" || true)
NIT=$(grep -c '\[NIT\]' "$OUT" || true)
```

If parse looks broken (output empty, or contains none of the markers AND is non-empty):
- `VERDICT: ESCALATE: azcodex output format unrecognized — see <OUT>`

Never silently PASS on parse failure.

## Output

```
azcodex review → .claude-pipeline/azcodex.md
  [CRITICAL]: 0
  [BLOCKING]: 1
  [NIT]:      3

VERDICT: FAIL: 1 blocking finding (see .claude-pipeline/azcodex.md)
```

PASS only if both CRIT and BLOCK are 0.
```

- [ ] **Step 2: Commit**

```bash
git add plugins/implement-pipeline/skills/azcodex-review/SKILL.md
git commit -m "plugin: azcodex-review specialist skill"
```

---

## Task 8: Slash commands

**Files:**
- Create: `plugins/implement-pipeline/commands/implement.md`
- Create: `plugins/implement-pipeline/commands/browser-test.md`
- Create: `plugins/implement-pipeline/commands/ux-review.md`
- Create: `plugins/implement-pipeline/commands/azcodex-review.md`

- [ ] **Step 1: Write `implement.md`**

```markdown
---
description: Run the hands-off implementation pipeline (implement → simplify → lint → browser → UX → azcodex → PR).
argument-hint: [path | "free text" | LIN-id | #issue | --resume | --from <stage> | --skip <stage,…> | --no-pr]
---

Run the implement-pipeline orchestrator on `$ARGUMENTS`.

Invoke the `implement-pipeline:implement-workflow` skill with the arguments. The skill handles input classification, state, gates, and PR handoff.

If $ARGUMENTS is empty, the orchestrator will resume an in-progress run, or pick the newest unfinished plan, or ask.
```

- [ ] **Step 2: Write `browser-test.md`**

```markdown
---
description: Browser-test changed UI routes on the current branch. Captures screenshots, console, network.
argument-hint: [base-branch] (default: main)
---

Run the browser-test gate against the current branch.

Invoke the `implement-pipeline:browser-test` skill. Pass `$ARGUMENTS` as BASE if provided.
```

- [ ] **Step 3: Write `ux-review.md`**

```markdown
---
description: UX + scope review on current diff. Design-system pass plus scope-discipline pass.
argument-hint: [spec-or-plan-path]
---

Run the UX review on the current branch.

Invoke the `implement-pipeline:ux-review` skill. If `$ARGUMENTS` is a path, pass it as SPEC_PATH/PLAN_PATH.
```

- [ ] **Step 4: Write `azcodex-review.md`**

```markdown
---
description: External code review via azcodex (codex --profile azure). Reads diff vs base.
argument-hint: [base-branch] (default: main)
---

Run azcodex external review.

Invoke the `implement-pipeline:azcodex-review` skill. Pass `$ARGUMENTS` as BASE if provided.
```

- [ ] **Step 5: Verify command frontmatter parses**

Run:

```bash
for f in plugins/implement-pipeline/commands/*.md; do
  head -5 "$f" | grep -q '^description:' || { echo "$f: missing description"; exit 1; }
done && echo OK
```

Expected: `OK`

- [ ] **Step 6: Commit**

```bash
git add plugins/implement-pipeline/commands/
git commit -m "plugin: slash commands for implement + standalone stages"
```

---

## Task 9: Failing test — `installClaude` previews plugin commands

**Files:**
- Modify: `install_test.go`

- [ ] **Step 1: Write the failing test**

Append to `install_test.go`:

```go
func TestInstallClaudePreviewMentionsPluginCommands(t *testing.T) {
	tmpHome := t.TempDir()
	t.Setenv("HOME", tmpHome)

	stdout, _, err := captureInstallClaude(t, nil)
	if err != nil {
		t.Fatalf("installClaude preview: %v", err)
	}
	if !strings.Contains(stdout, "claude plugin marketplace add") {
		t.Fatalf("preview should mention `claude plugin marketplace add`; got:\n%s", stdout)
	}
	if !strings.Contains(stdout, "claude plugin install implement-pipeline@domux") {
		t.Fatalf("preview should mention `claude plugin install implement-pipeline@domux`; got:\n%s", stdout)
	}
}
```

Also add this helper near the top of `install_test.go` (after imports — only one copy):

```go
// captureInstallClaude runs installClaude(args) capturing stdout/stderr.
func captureInstallClaude(t *testing.T, args []string) (string, string, error) {
	t.Helper()
	rOut, wOut, _ := os.Pipe()
	rErr, wErr, _ := os.Pipe()
	origOut, origErr := os.Stdout, os.Stderr
	os.Stdout, os.Stderr = wOut, wErr
	defer func() {
		os.Stdout, os.Stderr = origOut, origErr
	}()

	err := installClaude(args)

	wOut.Close()
	wErr.Close()
	outBytes, _ := io.ReadAll(rOut)
	errBytes, _ := io.ReadAll(rErr)
	return string(outBytes), string(errBytes), err
}
```

Ensure `"io"` is in the import block (it isn't — add it).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/pranav/projects/domux && go test -run TestInstallClaudePreviewMentionsPluginCommands ./...`
Expected: FAIL with message like `preview should mention 'claude plugin marketplace add'`.

- [ ] **Step 3: Commit the failing test**

```bash
git add install_test.go
git commit -m "test: failing — installClaude preview must mention plugin commands"
```

---

## Task 10: Implement plugin install in `installClaude`

**Files:**
- Modify: `install.go`

- [ ] **Step 1: Add plugin-install logic to `installClaude`**

Replace the current `installClaude` function body. Keep the existing settings-patch behavior. Append plugin handling.

In `install.go`, find:

```go
func installClaude(args []string) error {
	fs := flag.NewFlagSet("install claude", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	apply := fs.Bool("apply", false, "patch ~/.claude/settings.json")
	if err := fs.Parse(args); err != nil {
		return err
	}
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot get home directory: %w", err)
	}
	path := filepath.Join(homeDir, ".claude", "settings.json")
	next, err := patchedClaudeSettings(path)
	if err != nil {
		return err
	}
	if !*apply {
		data, _ := json.MarshalIndent(next, "", "  ")
		fmt.Printf("Would patch %s with domux hooks/statusLine:\n\n%s\n", path, data)
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", filepath.Dir(path), err)
	}
	if err := backupIfExists(path); err != nil {
		return err
	}
	data, err := json.MarshalIndent(next, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	if err := os.WriteFile(path, data, 0600); err != nil {
		return fmt.Errorf("cannot write %s: %w", path, err)
	}
	fmt.Printf("patched %s\n", path)
	return nil
}
```

Replace with:

```go
func installClaude(args []string) error {
	fs := flag.NewFlagSet("install claude", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	apply := fs.Bool("apply", false, "patch ~/.claude/settings.json + install implement-pipeline plugin")
	if err := fs.Parse(args); err != nil {
		return err
	}
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot get home directory: %w", err)
	}
	path := filepath.Join(homeDir, ".claude", "settings.json")
	next, err := patchedClaudeSettings(path)
	if err != nil {
		return err
	}
	marketplaceSource := claudePluginMarketplaceSource()
	if !*apply {
		data, _ := json.MarshalIndent(next, "", "  ")
		fmt.Printf("Would patch %s with domux hooks/statusLine:\n\n%s\n", path, data)
		printPluginInstallPlan(marketplaceSource)
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", filepath.Dir(path), err)
	}
	if err := backupIfExists(path); err != nil {
		return err
	}
	data, err := json.MarshalIndent(next, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	if err := os.WriteFile(path, data, 0600); err != nil {
		return fmt.Errorf("cannot write %s: %w", path, err)
	}
	fmt.Printf("patched %s\n", path)
	return runPluginInstall(marketplaceSource)
}

// claudePluginMarketplaceSource returns the path/repo to register as the
// domux marketplace. Prefers a local clone (detected via os.Executable walking
// up to find .claude-plugin/marketplace.json), falls back to GitHub.
func claudePluginMarketplaceSource() string {
	if local := detectLocalMarketplaceRoot(); local != "" {
		return local
	}
	return "pranav7/domux"
}

func detectLocalMarketplaceRoot() string {
	exe, err := os.Executable()
	if err != nil {
		return ""
	}
	dir := filepath.Dir(exe)
	for i := 0; i < 8; i++ {
		if fileExists(filepath.Join(dir, ".claude-plugin", "marketplace.json")) {
			return dir
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	// Also try the current working directory — common when running `go run` or
	// the freshly built binary from the repo root.
	if cwd, err := os.Getwd(); err == nil {
		if fileExists(filepath.Join(cwd, ".claude-plugin", "marketplace.json")) {
			return cwd
		}
	}
	return ""
}

func printPluginInstallPlan(source string) {
	if !commandExists("claude") {
		fmt.Println()
		fmt.Println("Would install implement-pipeline plugin, but `claude` CLI is not on PATH.")
		fmt.Println("Install Claude Code first: https://claude.com/claude-code")
		return
	}
	fmt.Println()
	fmt.Println("Would also run:")
	fmt.Printf("  claude plugin marketplace add %s\n", source)
	fmt.Println("  claude plugin install implement-pipeline@domux")
}

func runPluginInstall(source string) error {
	if !commandExists("claude") {
		fmt.Println()
		fmt.Println("warning: `claude` CLI not on PATH — skipping plugin install.")
		fmt.Println("Install Claude Code (https://claude.com/claude-code), then run:")
		fmt.Printf("  claude plugin marketplace add %s\n", source)
		fmt.Println("  claude plugin install implement-pipeline@domux")
		return nil
	}
	addCmd := exec.Command("claude", "plugin", "marketplace", "add", source)
	addCmd.Stdout = os.Stdout
	addCmd.Stderr = os.Stderr
	if err := addCmd.Run(); err != nil {
		return fmt.Errorf("claude plugin marketplace add: %w", err)
	}
	installCmd := exec.Command("claude", "plugin", "install", "implement-pipeline@domux")
	installCmd.Stdout = os.Stdout
	installCmd.Stderr = os.Stderr
	if err := installCmd.Run(); err != nil {
		return fmt.Errorf("claude plugin install: %w", err)
	}
	fmt.Println("installed implement-pipeline plugin")
	return nil
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cd /Users/pranav/projects/domux && go test -run TestInstallClaudePreviewMentionsPluginCommands ./...`
Expected: PASS

- [ ] **Step 3: Run full test suite to confirm no regression**

Run: `cd /Users/pranav/projects/domux && go test ./...`
Expected: PASS (all existing tests still green)

- [ ] **Step 4: Commit**

```bash
git add install.go install_test.go
git commit -m "install: register domux marketplace + install implement-pipeline plugin"
```

---

## Task 11: Failing test — skips plugin install when `claude` CLI missing

**Files:**
- Modify: `install_test.go`

- [ ] **Step 1: Write the failing test**

Append to `install_test.go`:

```go
func TestInstallClaudeSkipsPluginStepsWhenClaudeCliMissing(t *testing.T) {
	tmpHome := t.TempDir()
	t.Setenv("HOME", tmpHome)
	// Ensure `claude` is not findable on PATH.
	t.Setenv("PATH", "")

	stdout, _, err := captureInstallClaude(t, []string{"--apply"})
	if err != nil {
		t.Fatalf("installClaude --apply: %v", err)
	}
	if !strings.Contains(stdout, "`claude` CLI not on PATH") {
		t.Fatalf("expected warning about missing claude CLI; got:\n%s", stdout)
	}
	// Settings should still have been written even without claude on PATH.
	settingsPath := filepath.Join(tmpHome, ".claude", "settings.json")
	if !fileExists(settingsPath) {
		t.Fatalf("expected settings.json to be written even when claude CLI is missing")
	}
}
```

- [ ] **Step 2: Run test to verify behavior**

Run: `cd /Users/pranav/projects/domux && go test -run TestInstallClaudeSkipsPluginStepsWhenClaudeCliMissing ./...`
Expected: PASS (the runPluginInstall code path written in Task 10 already handles the missing-CLI case, so this test should pass — if it doesn't, the wording in the warning needs to match).

If FAIL: adjust the warning string in `runPluginInstall` to match the assertion (`fmt.Println("warning: \`claude\` CLI not on PATH …")`). The intent here is to lock in observable behavior with a test.

- [ ] **Step 3: Commit**

```bash
git add install_test.go
git commit -m "test: install skips plugin steps gracefully when claude CLI missing"
```

---

## Task 12: Manifest-validity tests

**Files:**
- Create: `manifest_test.go`

- [ ] **Step 1: Write the manifest tests**

`manifest_test.go`:

```go
package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestMarketplaceManifestIsValid(t *testing.T) {
	data, err := os.ReadFile(filepath.Join(".claude-plugin", "marketplace.json"))
	if err != nil {
		t.Fatalf("read marketplace.json: %v", err)
	}
	var m struct {
		Name                                 string `json:"name"`
		AllowCrossMarketplaceDependenciesOn []string `json:"allowCrossMarketplaceDependenciesOn"`
		Plugins []struct {
			Name        string `json:"name"`
			Source      string `json:"source"`
			Description string `json:"description"`
		} `json:"plugins"`
	}
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("parse marketplace.json: %v", err)
	}
	if m.Name != "domux" {
		t.Fatalf("marketplace name = %q, want \"domux\"", m.Name)
	}
	if len(m.Plugins) == 0 {
		t.Fatalf("marketplace has no plugins")
	}
	foundCross := false
	for _, dep := range m.AllowCrossMarketplaceDependenciesOn {
		if dep == "claude-plugins-official" {
			foundCross = true
		}
	}
	if !foundCross {
		t.Fatalf("allowCrossMarketplaceDependenciesOn must include 'claude-plugins-official'; got %v", m.AllowCrossMarketplaceDependenciesOn)
	}
	for _, p := range m.Plugins {
		if p.Name == "" || p.Source == "" {
			t.Fatalf("plugin entry incomplete: %+v", p)
		}
		// Source must resolve to an existing directory.
		if !dirExists(p.Source) {
			t.Fatalf("plugin source %q does not exist", p.Source)
		}
	}
}

func TestPluginManifestIsValid(t *testing.T) {
	path := filepath.Join("plugins", "implement-pipeline", ".claude-plugin", "plugin.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read plugin.json: %v", err)
	}
	var p struct {
		Name        string `json:"name"`
		Version     string `json:"version"`
		Description string `json:"description"`
		Dependencies []struct {
			Name        string `json:"name"`
			Marketplace string `json:"marketplace"`
		} `json:"dependencies"`
	}
	if err := json.Unmarshal(data, &p); err != nil {
		t.Fatalf("parse plugin.json: %v", err)
	}
	if p.Name != "implement-pipeline" {
		t.Fatalf("plugin name = %q, want \"implement-pipeline\"", p.Name)
	}
	if p.Version == "" {
		t.Fatalf("plugin version is empty")
	}
	wantDeps := map[string]bool{
		"code-simplifier": false,
		"frontend-design": false,
		"typescript-lsp":  false,
		"pyright-lsp":     false,
	}
	for _, dep := range p.Dependencies {
		if dep.Marketplace != "claude-plugins-official" {
			t.Fatalf("dependency %q has marketplace %q, want \"claude-plugins-official\"", dep.Name, dep.Marketplace)
		}
		if _, ok := wantDeps[dep.Name]; ok {
			wantDeps[dep.Name] = true
		}
	}
	for name, found := range wantDeps {
		if !found {
			t.Fatalf("plugin manifest missing dependency on %q", name)
		}
	}
}

func TestPluginSkillsHaveRequiredFrontmatter(t *testing.T) {
	skills := []string{
		"plugins/implement-pipeline/skills/implement-workflow/SKILL.md",
		"plugins/implement-pipeline/skills/browser-test/SKILL.md",
		"plugins/implement-pipeline/skills/ux-review/SKILL.md",
		"plugins/implement-pipeline/skills/azcodex-review/SKILL.md",
	}
	for _, s := range skills {
		data, err := os.ReadFile(s)
		if err != nil {
			t.Fatalf("read %s: %v", s, err)
		}
		text := string(data)
		if len(text) < 10 || text[:4] != "---\n" {
			t.Fatalf("%s: missing frontmatter", s)
		}
		// Cheap sanity check: name: and description: present in the head.
		head := text
		if len(head) > 500 {
			head = head[:500]
		}
		if !contains(head, "name:") || !contains(head, "description:") {
			t.Fatalf("%s: frontmatter must declare name and description", s)
		}
	}
}

func contains(haystack, needle string) bool {
	for i := 0; i+len(needle) <= len(haystack); i++ {
		if haystack[i:i+len(needle)] == needle {
			return true
		}
	}
	return false
}
```

- [ ] **Step 2: Run tests**

Run: `cd /Users/pranav/projects/domux && go test -run 'TestMarketplaceManifestIsValid|TestPluginManifestIsValid|TestPluginSkillsHaveRequiredFrontmatter' ./...`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add manifest_test.go
git commit -m "test: validate marketplace + plugin manifests + skill frontmatter"
```

---

## Task 13: Regression test — `start-task` still installed (sanity)

**Files:**
- Modify: `install_test.go`

- [ ] **Step 1: Inspect current claude-install path for start-task references**

Run: `cd /Users/pranav/projects/domux && grep -n start-task install.go`

If `start-task.md` is no longer wired up in install.go (the spec mentions it but the current code may have been refactored), **skip this task** and proceed to Task 14. Document the skip in the implementation notes.

- [ ] **Step 2: If start-task IS still installed, add regression test**

Append to `install_test.go`:

```go
func TestPatchedClaudeSettingsStillIncludesExistingHooks(t *testing.T) {
	path := filepath.Join(t.TempDir(), "settings.json")
	settings, err := patchedClaudeSettings(path)
	if err != nil {
		t.Fatalf("patchedClaudeSettings: %v", err)
	}
	events, _ := settings["hooks"].(map[string]any)
	for _, want := range []string{"SessionStart", "PreToolUse", "Stop"} {
		if _, ok := events[want]; !ok {
			t.Fatalf("expected %s hook to still be installed after plugin work", want)
		}
	}
}
```

- [ ] **Step 3: Run test**

Run: `cd /Users/pranav/projects/domux && go test -run TestPatchedClaudeSettingsStillIncludesExistingHooks ./...`
Expected: PASS

- [ ] **Step 4: Commit (only if test added)**

```bash
git add install_test.go
git commit -m "test: regression — existing claude hooks survive plugin install path"
```

---

## Task 14: Smoke-install Makefile target

**Files:**
- Modify: `Makefile`

- [ ] **Step 1: Read current Makefile**

Run: `cat /Users/pranav/projects/domux/Makefile`

- [ ] **Step 2: Append smoke-install target**

Append to `Makefile`:

```make
.PHONY: smoke-install-claude
smoke-install-claude: build
	@echo "==> Previewing claude install (no --apply)"
	./domux install claude
	@echo
	@echo "==> Apply step is manual — run \`./domux install claude --apply\` yourself to write."
```

If a `build` target doesn't already exist in the Makefile, depend on `domux` (or whatever the existing build target's name is) instead. Inspect first.

- [ ] **Step 3: Verify**

Run: `cd /Users/pranav/projects/domux && make smoke-install-claude`
Expected: prints the preview output (settings JSON + `Would also run: claude plugin marketplace add …`).

- [ ] **Step 4: Commit**

```bash
git add Makefile
git commit -m "make: smoke-install-claude target for manual install verification"
```

---

## Task 15: User-facing usage doc

**Files:**
- Create: `docs/implement-pipeline.md`

- [ ] **Step 1: Write the usage doc**

```markdown
# Implement Pipeline

A Claude Code plugin shipped from the domux marketplace. Runs implement →
simplify → lint → browser test → UX review → external code review → PR.

## Install

```bash
domux install claude --apply
```

This patches `~/.claude/settings.json` (existing behavior) and then runs:

```bash
claude plugin marketplace add <local-path-or-pranav7/domux>
claude plugin install implement-pipeline@domux
```

`claude` CLI must be on PATH. Without it, the install prints the commands and
skips the plugin step — patching of settings still happens.

## Usage

```
/implement                           # resume or pick newest unfinished plan
/implement docs/superpowers/plans/foo.md
/implement "fix the broken signup button"
/implement LIN-1234
/implement #42
/implement --resume
/implement --from browser
/implement --skip ux,azcodex
/implement --no-pr
```

Standalone gates:

```
/browser-test [base-branch]
/ux-review    [spec-or-plan-path]
/azcodex-review [base-branch]
```

## Pipeline stages

| # | Stage         | Source                                        | Skip rule                              |
|---|---------------|-----------------------------------------------|----------------------------------------|
| 1 | Implement     | superpowers:subagent-driven-development       | Never                                   |
| 2 | Simplify      | code-simplifier:code-simplifier agent          | No source files in diff                 |
| 3 | Lint auto-fix | Inline Bash (ruff/biome/eslint/prettier)       | No .py/.ts/.tsx/.js/.jsx in diff        |
| 4 | Browser test  | implement-pipeline:browser-test                 | No UI files in diff                     |
| 5 | UX + scope    | implement-pipeline:ux-review                    | Same as 4                               |
| 6 | azcodex       | implement-pipeline:azcodex-review               | Never                                   |
| 7 | Address       | general-purpose subagent on findings            | No findings                             |
| 8 | PR + babysit  | /commit-push-pr + /loop 5m /babysit             | --no-pr                                 |

## Artifacts

`.claude-pipeline/` (gitignored):

```
.claude-pipeline/
  state.json
  azcodex.md
  artifacts/browser/{*.png,*.gif,console.log}
  findings/ux.md
```

## Dependencies (auto-installed)

- claude-plugins-official:code-simplifier
- claude-plugins-official:frontend-design
- claude-plugins-official:typescript-lsp
- claude-plugins-official:pyright-lsp

## Cleanup of legacy commands

After installing this plugin, delete `~/dotfiles/claude/commands/browser-test.md`
to avoid shadowing the plugin command. The other dotfiles commands
(`babysit.md`, `commit-push-pr.md`, `check-pr-comments.md`) stay — the
pipeline invokes them by name.
```

- [ ] **Step 2: Commit**

```bash
git add docs/implement-pipeline.md
git commit -m "docs: implement-pipeline usage"
```

---

## Task 16: README pointer

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Read current README**

Run: `cat /Users/pranav/projects/domux/README.md`

- [ ] **Step 2: Add a one-section pointer**

Add a new section near the top (after the "Install" or first usage section, wherever makes sense given current structure):

```markdown
## /implement pipeline

`domux install claude --apply` also installs the `implement-pipeline` Claude Code plugin:

- `/implement` — hands-off pipeline (implement → simplify → lint → browser → UX → azcodex review → PR)
- `/browser-test`, `/ux-review`, `/azcodex-review` — individually invokable gates

See [docs/implement-pipeline.md](docs/implement-pipeline.md) for details.
```

Match the heading style of the existing README. If the README uses `###` instead of `##`, downshift.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "readme: link to implement-pipeline plugin"
```

---

## Task 17: End-to-end smoke check + push branch

**Files:** none modified — verification step.

- [ ] **Step 1: Full test suite green**

Run: `cd /Users/pranav/projects/domux && go test ./...`
Expected: all PASS.

- [ ] **Step 2: `go build ./...` green**

Run: `cd /Users/pranav/projects/domux && go build ./...`
Expected: clean.

- [ ] **Step 3: Manual preview check**

Run: `cd /Users/pranav/projects/domux && go run . install claude`
Expected: prints the settings JSON it would write, then `Would also run: claude plugin marketplace add <path>` and `claude plugin install implement-pipeline@domux`.

- [ ] **Step 4: JSON validity sweep**

Run:

```bash
cd /Users/pranav/projects/domux && python3 -c "
import json, pathlib
for p in pathlib.Path('.').rglob('*.json'):
  if '.git' in p.parts or 'dist' in p.parts: continue
  json.loads(p.read_text())
print('OK')
"
```

Expected: `OK`.

- [ ] **Step 5: Push branch (no PR yet — that's the user's call)**

Run: `cd /Users/pranav/projects/domux && git push -u origin add-implement-pipeline`
Expected: branch published.

- [ ] **Step 6: Report to user**

Tell the user:
- Branch pushed.
- All tests pass.
- Manual smoke (`./domux install claude --apply` against a real `claude` CLI) is the only thing the test suite doesn't cover — call that out.
- PR is theirs to open (or `/commit-push-pr`).

---

## Self-review checklist (already performed during plan writing)

1. **Spec coverage:**
   - marketplace.json + plugin.json → Task 1.
   - File layout under `plugins/implement-pipeline/` → Tasks 3–8.
   - Pipeline stages 1–8 → Task 3 (orchestrator) + Tasks 4–7 (prompts + specialist skills).
   - Gate VERDICT contract → built into Tasks 5, 6, 7.
   - State file `.claude-pipeline/state.json` → Task 3 + Task 2 (gitignore).
   - Entry-point detection → Task 3.
   - Skip rules → Task 3.
   - Lint detail → Task 3 + Task 4 (lint-prompt.md).
   - Login-wall protocol → Task 5.
   - azcodex parsing → Task 7.
   - install.go integration → Tasks 9–11.
   - Tests `TestInstallClaudePreview…`, `TestInstallClaudeSkips…`, `TestPluginManifestIsValid`, `TestMarketplaceManifestIsValid`, `TestStartTaskStillInstalled` → Tasks 9, 11, 12, 13.
   - Rollout / Migration / Versioning → Task 15 (docs).
   - README mention → Task 16.

2. **Placeholder scan:** No "TBD", "TODO", or "implement later" in any step. Every code block is concrete.

3. **Type consistency:** `claudePluginMarketplaceSource`, `detectLocalMarketplaceRoot`, `printPluginInstallPlan`, `runPluginInstall`, `captureInstallClaude` are all referenced consistently across Tasks 9–11. Helper `fileExists`, `dirExists`, `commandExists` already exist in `install.go`.

4. **Out-of-spec items deferred:** Hooks-based enforcement explicitly not built. `/simplify` and `/lint-fix` commands explicitly not built. Open questions in the spec (markdown vs JSON for UX findings, stronger model for security findings) settled inline in Task 6 (markdown) and Task 3 (no model upgrade — escalate instead).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-25-implement-workflow.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
