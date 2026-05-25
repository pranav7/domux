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
