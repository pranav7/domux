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
