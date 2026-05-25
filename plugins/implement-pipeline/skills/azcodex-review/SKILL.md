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
