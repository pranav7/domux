---
name: codex-review
description: Use when running /codex-review, when you want an external second-opinion code review on the current changes from a different model family (Codex / GPT-5.5 on Azure), or as the external-lens leg of /implement's analyze phase. Surfaces BLOCKER / IMPORTANT / NON-BLOCKER findings with Claude's own judgement layered on top.
---

# /codex-review — external review via Codex (Azure / GPT-5.5)

Review the current changes through **Codex** — a different model family than Claude — then layer your own judgement on top. A second family reviewing the same diff catches failure modes that same-family review misses. Brief Codex on the problem and the approach first, so it can also give a second opinion on whether the solution is the right one — not just hunt for code-level bugs. Codex is one more reviewer whose findings you verify, **not** ground truth.

`codex` is the user's shell alias for `codex --profile azure` (Codex CLI, GPT-5.5, Azure endpoint), and the alias IS loaded in Claude's shell. Call plain `codex …` and NEVER pass `--profile azure` yourself — the alias already injects it, and a doubled flag makes the CLI exit with `the argument '--profile <CONFIG_PROFILE_V2>' cannot be used multiple times`. (Fallback: only if `type codex` shows no alias — e.g. on another machine — add `--profile azure` back.)

## How `codex review` actually behaves (verified against codex-cli 0.141.0)

Two hard constraints shape every command below — both confirmed by testing, not docs:

1. **A custom prompt and a scope flag are mutually exclusive.** `codex review` takes *either* a positional `[PROMPT]` *or* one of `--uncommitted` / `--base <BRANCH>` / `--commit <SHA>` — never both. Passing both exits with an arg error and runs no review. So the brief can only ride along on the prompt-only form.
2. **Prompt-only mode reviews ONLY the uncommitted working tree** (staged + unstaged + untracked). It does *not* auto-detect committed branch work — on a clean tree it returns "no current changes, 0 findings." To review committed work you must use a scope flag, which means you cannot also pass the brief.
3. **`codex review` is NOT read-only under the user's config.** The base config sets `sandbox_mode = "workspace-write"`, and `review` will *edit files in place* to apply its own suggestions. Always pass `--sandbox read-only` so it can read the diff but cannot touch the tree. (Verified: with `--sandbox read-only` the review still runs and reports findings, but leaves every file byte-for-byte unchanged.)

Consequence: to get the solution-level second opinion (the brief), review **before committing** while the tree is dirty. Once work is committed, you can still review it, but only code-level and without the brief.

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

The scope decides which of the two forms you run (see the constraints above):

- **Working tree dirty** (`git status --porcelain` non-empty) → **Path A (prompt-only)**. Review the uncommitted tree *with the brief* — the full solution + code review. This is the preferred path; review before committing whenever you can.
- **Clean but ahead of base** → **Path B (scope flag)**. Review the branch with `--base "$BASE"`, where `BASE=$(git symbolic-ref refs/remotes/origin/HEAD | sed 's@^refs/remotes/origin/@@')` (default `main`). The brief cannot be passed here — this is a code-level review only. `git fetch origin` first so the base ref isn't stale.
- **A single commit** → **Path B** with `--commit "$SHA"`.
- **Nothing to review** → say so and stop.

## 3. Run Codex

Always pass `--sandbox read-only` (see constraint 3) so the review can read but never edit the tree. Redirect to a log file: Codex prepends a large model-list JSON and a `failed to refresh available models` line that crowds out its summary, so capture to a file and read the tail rather than trusting inline stdout.

**Path A — uncommitted work, with the brief (preferred).** The prompt leads with the brief from step 1, then asks Codex to weigh in on both the approach and the code:

```bash
mkdir -p .implement
PROMPT="You are giving a second opinion on a change. Here is the author's brief on what they're solving and how:

$BRIEF

Review the change on two levels. First, the SOLUTION: does this approach actually solve the stated problem? Is there a simpler, safer, or more correct way? Flag any design flaw, wrong abstraction, missed edge case, or mismatch between the brief and the diff. Second, the CODE: correctness bugs, security issues, regressions, and spec compliance.

Tag every finding with exactly one of [BLOCKER] | [IMPORTANT] | [NON-BLOCKER]: [BLOCKER]=data loss / security / breaks prod / wrong approach / must-fix before merge; [IMPORTANT]=real bug, risk, or design concern worth fixing now; [NON-BLOCKER]=nit, style, or follow-up. End with a one-line count."
codex --sandbox read-only review "$PROMPT" > .implement/codex-review.log 2>&1
```

**Path B — committed work, no brief (code-level only).** A scope flag forbids a custom prompt, so Codex uses its built-in review instructions:

```bash
codex --sandbox read-only review --base "$BASE"  > .implement/codex-review.log 2>&1
# single commit instead: codex --sandbox read-only review --commit "$SHA" > .implement/codex-review.log 2>&1
```

When you take Path B, say so in the final report: **"Author's brief was NOT passed to Codex — findings are code-level only, not solution-level."**

Cap the run if a timeout tool exists — `gtimeout 420 codex …` (plain `timeout` is usually absent on macOS, so this is best-effort, not required). On timeout → report `codex review timed out — inconclusive`.

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
/codex-review (scope: uncommitted+brief | vs <base>, code-level only | commit <sha>, code-level only)

BLOCKER
- <finding> — codex: <claim>; you: <confirm + why>  (file:line)

IMPORTANT
- …

NON-BLOCKER
- …

Summary: <n> blocker · <n> important · <n> non-blocker — PASS if 0 blocker & 0 important, else NEEDS FIXES
```

For Path B, the scope line must flag that the brief wasn't passed (code-level only). Report inconclusive Codex runs as `INCONCLUSIVE`, not `PASS`.
