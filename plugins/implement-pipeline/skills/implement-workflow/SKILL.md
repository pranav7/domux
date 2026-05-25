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
