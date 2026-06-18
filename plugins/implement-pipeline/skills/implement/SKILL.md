---
name: implement
description: Hands-off implementation pipeline. Picks up a spec / plan / issue / free-text task and runs it end to end — implement → simplify → lint → verify → review → PR — by reusing Claude's built-in skills. Use when the user runs /implement or asks to "implement X end to end".
---

# /implement — implementation pipeline

Take one unit of work from idea to PR, hands-off. Every stage reuses a skill Claude already ships; this orchestrator only sequences them, judges their results, and persists enough state to resume. Don't reinvent simplify/verify/review — invoke the built-ins.

## Arguments

| Arg | Behavior |
|---|---|
| (none) | Resume an in-progress run; else the newest unfinished plan in `docs/superpowers/plans/`; else ask. |
| `<path.md>` | spec (frontmatter `status: spec`, or under `/specs/`) → `superpowers:writing-plans` first, then run the plan. plan → run directly. |
| `"free text"` | Small task — skip plan-writing, implement directly. |
| `LIN-<id>` | `/fix-linear-issue LIN-<id>` to load context, then run on the resulting diff. |
| `#<n>` | `gh issue view <n> --json title,body` → treat as free text. |
| `--resume` | Read `.implement/state.json`, skip completed stages. |
| `--skip <a,b>` | Skip named stages or analyze legs (e.g. `--skip lint,codex-review`). |
| `--no-pr` | Stop after review; don't open a PR. |

## Before a new run (skip when `--resume`)

1. `git status --porcelain` must be empty — else refuse: "commit or stash first, or pass --resume".
2. `BASE = git symbolic-ref refs/remotes/origin/HEAD | sed 's@^refs/remotes/origin/@@'` (default `main`).
3. `mkdir -p .implement` (already gitignored). Write `state.json` (see below).

Recompute `CHANGED = git diff --name-only "$BASE"...HEAD` before each stage that needs it.

## Stages

Run in order. Skip a stage if its trigger doesn't fire or it's listed in `--skip`. After each, **judge the invoked skill's own output** — the built-ins don't emit a verdict line, so read what they report and decide pass / stop.

1. **Implement** — plan → `superpowers:subagent-driven-development` against it; spec → `superpowers:writing-plans` first. Free-text → implement directly (lean on `superpowers:test-driven-development` when it fits). Commit the work.
2. **Simplify** — invoke the built-in **`/simplify`** skill (`Skill: simplify`). Quality-only pass over the diff; no behavior change.
3. **Lint** — only if `CHANGED` has lintable files. Auto-fix with the project's own tools, commit `chore: lint`, then re-check clean:
   - `*.py` + `pyproject.toml` w/ ruff → `ruff check --fix . && ruff format .`
   - `*.ts/tsx/js/jsx` + biome → `npx --no-install biome check --apply .`; else eslint+prettier on the changed files.
   Residual errors after the fix → stop, report.
4. **Analyze** *(parallel fan-out)* — once lint has committed, the tree is frozen, so the read-only checks below are independent and run **concurrently**. Dispatch them as named sub-agents in a single message (see *Running the analyze fan-out*). Wait for all, then synthesize one finding list — dedupe overlaps, let your own read win ties:
   - **verify** → `Skill: verify` — run the app and exercise the change (frontend in the browser, backend via API). Report anything broken.
   - **code-review** → `Skill: code-review` on the diff — your own correctness/quality lens.
   - **codex-review** → `Skill: codex-review` — external second opinion from a different model family (Codex / GPT-5.5 on Azure). Skips itself silently if `codex` is absent.
   - **design** *(only if UI changed* — `*.tsx/jsx/html/css` or under `pages/ app/ routes/ components/`)* → `frontend-design:frontend-design` for a design pass; scope-check that every new UI element traces to a requirement (flag dead-end buttons / out-of-scope additions).
5. **Address** — if the synthesized findings include any **BLOCKER** or **IMPORTANT** item (or `verify` found a breakage): apply the minimal fix for each, add/adjust tests, then re-run lint + a fresh **Analyze** pass over the touched code. Max 2 passes, then escalate to the user. NON-BLOCKERs only if trivial.
6. **PR** — `/commit-push-pr` (fallback `gh pr create -f`), then `/loop 5m /babysit` to watch CI and address review comments. Skipped when `--no-pr`.

### Running the analyze fan-out

The legs are read-only and share no state, so they parallelize cleanly. Concurrency *and* a colour-coded display come from the same move: **one `Agent` call per leg, all in the same message** — concurrent dispatches run in parallel and the TUI renders each as its own colour-coded lane. Name the agents so the colours map to legs: `verify`, `code-review`, `codex-review`, `design`.

Each sub-agent prompt must:
- State the leg's single job, the skill to invoke (`Skill: <name>`), and pass `BASE` + any spec/plan path.
- Be **read-only**: do not edit files or commit — only return findings. (Stage 5 is the only thing that mutates the tree, in the main thread, one fix at a time.)
- Return a compact list tagged **BLOCKER / IMPORTANT / NON-BLOCKER** so synthesis is mechanical.

If only one leg would actually run (no UI, `codex` absent, or `--skip` trims the rest), invoke it inline — no fan-out needed.

## State

`.implement/state.json` (gitignored), written atomically (`.tmp` + rename), one update per stage:

```json
{ "run_id": "<ISO-8601>", "input": "<arg>", "base": "main", "completed": ["implement", "simplify"] }
```

`--resume` reads `completed` and starts at the first stage not in it.

## Final summary

```
/implement <run_id>
  base:    <base>
  stages:  implement✓ simplify✓ lint✓ analyze✓ address(0) pr✓
  analyze: verify✓ code-review✓ codex-review✓ design–
  PR:      <url | —>
```
