---
description: Refactor / simplify the current diff via the code-simplifier subagent. Preserves behavior.
argument-hint: [extra guidance, optional]
---

Dispatch the `code-simplifier:code-simplifier` subagent on the current uncommitted diff.

Use the Agent tool with `subagent_type: "code-simplifier:code-simplifier"`. Tell it to:

- Focus on recently modified code (the working-tree diff vs HEAD).
- Preserve all behavior — no functional changes.
- Apply project conventions from `CLAUDE.md` if present.

If `$ARGUMENTS` is non-empty, forward it to the subagent as additional guidance (e.g. "only touch picker.go", "prefer table-driven tests", etc.).
