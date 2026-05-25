---
description: Run the hands-off implementation pipeline (implement → simplify → lint → browser → UX → azcodex → PR).
argument-hint: [path | "free text" | LIN-id | #issue | --resume | --from <stage> | --skip <stage,…> | --no-pr]
---

Run the implement-pipeline orchestrator on `$ARGUMENTS`.

Invoke the `implement-pipeline:implement-workflow` skill with the arguments. The skill handles input classification, state, gates, and PR handoff.

If $ARGUMENTS is empty, the orchestrator will resume an in-progress run, or pick the newest unfinished plan, or ask.
