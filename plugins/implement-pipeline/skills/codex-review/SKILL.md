---
name: codex-review
description: Use when running /codex-review, when you want an external second-opinion code review on the current changes from a different model family (Codex / GPT-5.5 on Azure), or as the external-lens leg of /implement's analyze phase. Surfaces BLOCKER / IMPORTANT / NON-BLOCKER findings with Claude's own judgement layered on top.
---

# /codex-review — external review via Codex (Azure / GPT-5.5)

Review the current changes through **Codex** — a different model family than Claude — then layer your own judgement on top. A second family reviewing the same diff catches failure modes that same-family review misses. Brief Codex on the problem and the approach first, so it can also give a second opinion on whether the solution is the right one — not just hunt for code-level bugs. Codex is one more reviewer whose findings you verify, **not** ground truth.

`azcodex` is the user's shell alias for `codex --profile azure` (Codex CLI, GPT-5.5, Azure endpoint). Aliases don't exist in non-interactive shells, so always call the full form: `codex --profile azure …`.

## Preconditions

- `command -v codex` must succeed. If not: report `codex not installed — skipping external review` and stop. (Inside /implement this is a silent skip, never a failure.)

## 1. Write the brief (so Codex reviews the solution, not just the syntax)

Codex only sees the diff — it can't see the problem you set out to solve or why you chose this approach. Without that context it can only nitpick code. Give it a short brief so it can also judge whether the solution itself is sound.

Assemble a tight paragraph (3–6 sentences) covering:

- **Problem** — what this change is trying to accomplish (the task, bug, or goal).
- **Approach** — the strategy you took, and any notable alternative you considered and rejected.
- **Constraints** — anything that bounds the solution (performance, compatibility, scope, deadline).
- **Where you want a second opinion** — the parts of the design you're least sure about.

Pull this from the conversation / spec / plan / issue that prompted the review — do not invent it. If you genuinely have no context (e.g. reviewing an unfamiliar diff blind), say so in the brief rather than fabricating intent. Store it in a shell variable for the next step:

```bash
BRIEF='Problem: … Approach: … Constraints: … Unsure about: …'
```

## 2. Pick the scope

- Working tree dirty (`git status --porcelain` non-empty) → review uncommitted work with `--uncommitted`.
- Clean but ahead of base → review the branch with `--base "$BASE"`, where `BASE = git symbolic-ref refs/remotes/origin/HEAD | sed 's@^refs/remotes/origin/@@'` (default `main`).
- Nothing to review → say so and stop.

## 3. Run Codex (read-only)

`codex review` is read-only by design — it never edits or commits. Redirect to a log file: Codex sometimes prepends a large model-list JSON and a `failed to refresh available models` line that can crowd out its summary, so capture to a file and read the tail rather than trusting inline stdout.

The prompt leads with the brief from step 1, then asks Codex to weigh in on both the approach and the code:

```bash
mkdir -p .implement
PROMPT="You are giving a second opinion on a change. Here is the author's brief on what they're solving and how:

$BRIEF

Review the change on two levels. First, the SOLUTION: does this approach actually solve the stated problem? Is there a simpler, safer, or more correct way? Flag any design flaw, wrong abstraction, missed edge case, or mismatch between the brief and the diff. Second, the CODE: correctness bugs, security issues, regressions, and spec compliance.

Tag every finding with exactly one of [BLOCKER] | [IMPORTANT] | [NON-BLOCKER]: [BLOCKER]=data loss / security / breaks prod / wrong approach / must-fix before merge; [IMPORTANT]=real bug, risk, or design concern worth fixing now; [NON-BLOCKER]=nit, style, or follow-up. End with a one-line count."
codex --profile azure review --uncommitted "$PROMPT" > .implement/codex-review.log 2>&1
# clean branch instead: codex --profile azure review --base "$BASE" "$PROMPT" > .implement/codex-review.log 2>&1
```

Cap the run where `timeout` is available (`timeout 420 codex …`). On timeout → report `codex review timed out — inconclusive`.

## 4. Read the result

Read the **tail** of `.implement/codex-review.log`, skipping the leading JSON / model-list noise. If the tail has no findings and none of the tags appear → report `codex review inconclusive (no findings block — see .implement/codex-review.log)`. **Never silently PASS on an unparseable result.**

## 5. Layer your judgement (the point of this skill)

Codex's tags are input, not the verdict. Read the actual diff and rule on each finding yourself — including any approach-level concern Codex raised against the brief:

- **Confirm** — you can see the bug/risk, or you agree the approach is flawed → keep its bucket.
- **Downgrade / drop** — false positive, out of scope, already handled, or a design objection that doesn't hold given the constraints in the brief → move it down a bucket or discard with a one-line reason.
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
