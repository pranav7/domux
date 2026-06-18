---
name: codex-review
description: Use when running /codex-review, when you want an external second-opinion code review on the current changes from a different model family (Codex / GPT-5.5 on Azure), or as the external-lens leg of /implement's analyze phase. Surfaces BLOCKER / IMPORTANT / NON-BLOCKER findings with Claude's own judgement layered on top.
---

# /codex-review — external review via Codex (Azure / GPT-5.5)

Review the current changes through **Codex** — a different model family than Claude — then layer your own judgement on top. A second family reviewing the same diff catches failure modes that same-family review misses. Codex is one more reviewer whose findings you verify, **not** ground truth.

`azcodex` is the user's shell alias for `codex --profile azure` (Codex CLI, GPT-5.5, Azure endpoint). Aliases don't exist in non-interactive shells, so always call the full form: `codex --profile azure …`.

## Preconditions

- `command -v codex` must succeed. If not: report `codex not installed — skipping external review` and stop. (Inside /implement this is a silent skip, never a failure.)

## 1. Pick the scope

- Working tree dirty (`git status --porcelain` non-empty) → review uncommitted work with `--uncommitted`.
- Clean but ahead of base → review the branch with `--base "$BASE"`, where `BASE = git symbolic-ref refs/remotes/origin/HEAD | sed 's@^refs/remotes/origin/@@'` (default `main`).
- Nothing to review → say so and stop.

## 2. Run Codex (read-only)

`codex review` is read-only by design — it never edits or commits. Redirect to a log file: Codex sometimes prepends a large model-list JSON and a `failed to refresh available models` line that can crowd out its summary, so capture to a file and read the tail rather than trusting inline stdout.

```bash
mkdir -p .implement
PROMPT='Review these changes for correctness bugs, security issues, regressions, and spec compliance. Tag every finding with exactly one of [BLOCKER] | [IMPORTANT] | [NON-BLOCKER]: [BLOCKER]=data loss / security / breaks prod / must-fix before merge; [IMPORTANT]=real bug or risk worth fixing now; [NON-BLOCKER]=nit, style, or follow-up. End with a one-line count.'
codex --profile azure review --uncommitted "$PROMPT" > .implement/codex-review.log 2>&1
# clean branch instead: codex --profile azure review --base "$BASE" "$PROMPT" > .implement/codex-review.log 2>&1
```

Cap the run where `timeout` is available (`timeout 420 codex …`). On timeout → report `codex review timed out — inconclusive`.

## 3. Read the result

Read the **tail** of `.implement/codex-review.log`, skipping the leading JSON / model-list noise. If the tail has no findings and none of the tags appear → report `codex review inconclusive (no findings block — see .implement/codex-review.log)`. **Never silently PASS on an unparseable result.**

## 4. Layer your judgement (the point of this skill)

Codex's tags are input, not the verdict. Read the actual diff and rule on each finding yourself:

- **Confirm** — you can see the bug/risk in the code → keep its bucket.
- **Downgrade / drop** — false positive, out of scope, or already handled → move it down a bucket or discard with a one-line reason.
- **Upgrade** — Codex under-rated it → move it up.

Add anything Codex missed that you spot while checking. Re-bucket into final BLOCKER / IMPORTANT / NON-BLOCKER.

## Output

```
/codex-review (scope: uncommitted | vs <base>)

BLOCKER
- <finding> — codex: <claim>; you: <confirm + why>  (file:line)

IMPORTANT
- …

NON-BLOCKER
- …

Summary: <n> blocker · <n> important · <n> non-blocker — PASS if 0 blocker & 0 important, else NEEDS FIXES
```

Report inconclusive Codex runs as `INCONCLUSIVE`, not `PASS`.
