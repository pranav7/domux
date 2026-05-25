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
