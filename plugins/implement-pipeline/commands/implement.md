---
description: Hands-off implementation pipeline — implement → simplify → lint → verify → review → PR, reusing Claude's built-in skills.
argument-hint: [path | "free text" | LIN-id | #issue | --resume | --skip <stages> | --no-pr]
---

Run the implementation pipeline on `$ARGUMENTS` by invoking the `implement-pipeline:implement-workflow` skill. It classifies the input, sequences the stages — reusing the built-in `/simplify`, `/verify`, and `/code-review` skills — judges each, and hands off to a PR.

Empty `$ARGUMENTS` → resume an in-progress run, else pick the newest unfinished plan, else ask.
