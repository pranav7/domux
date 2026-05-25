---
name: browser-test
description: Spin up a local dev server, exercise UI routes touched by the current diff in a real browser, capture screenshots/console/network logs. Use when the user runs /browser-test or as stage 4 of /implement.
---

# Browser Test

Exercise the primary flows on each UI route changed by the current diff (vs base branch). Capture artifacts to `.claude-pipeline/artifacts/browser/`.

## Inputs

If args provided, parse `BASE`, `SPEC_PATH`, `PLAN_PATH`, `CHANGED_UI_FILES`, `ARTIFACTS_DIR`. Otherwise:

- BASE: `git symbolic-ref refs/remotes/origin/HEAD | sed 's@^refs/remotes/origin/@@'` (default `main`).
- CHANGED_UI_FILES: `git diff --name-only "$BASE"...HEAD | grep -E '\.(tsx|jsx|html|css)$|^(pages|app|routes|components)/'`.
- ARTIFACTS_DIR: `.claude-pipeline/artifacts/browser/`.

## Procedure

1. **Infer routes.** From changed files, map to URL paths. (e.g. `app/settings/page.tsx` → `/settings`; `pages/users/[id].tsx` → `/users/123`.) If unclear, ask the user.

2. **Ensure dev server is running.**
   - `lsof -nP -iTCP -sTCP:LISTEN | grep -E ':(3000|5173|8080|4173)\s'` to check common ports.
   - If nothing's listening, look at `package.json` scripts for `dev|start|serve` and start it in background: `npm run dev > .claude-pipeline/dev-server.log 2>&1 &` (or yarn/pnpm equivalent based on lockfile). Wait up to 30s for port to open.

3. **Login wall.** Try `.env.test` creds if present (`TEST_USER`, `TEST_PASS`). If login fails or no creds:
   - Slack-DM `<@U0ADJBVPGUC>` in `#all_` (channel `C0ACZAG5QVD`) with:
     ```
     <@U0ADJBVPGUC> browser test waiting on login at <localhost-url>. React with ✅ when done.
     ```
   - Spawn a background subagent via `Agent` tool that polls thread + channel history with exponential backoff (1m, 3m, 9m). Resume when ✅ is seen.
   - If 13 minutes pass with no ✅: `VERDICT: ESCALATE: login-wall timeout`.

4. **Exercise each route.** Use the `claude-in-chrome` MCP tools (already on Pranav's system):
   - Navigate (`mcp__claude-in-chrome__navigate`).
   - Capture screenshot via `mcp__claude-in-chrome__gif_creator` (start recording before interaction, end after).
   - Read console messages (`mcp__claude-in-chrome__read_console_messages`); look for errors/warnings introduced by the change.
   - For each interactive element on the page that the diff touches, click/fill it; capture before/after screenshots.
   - Save artifacts as `{route-slug}.png`, `{route-slug}.gif`, `{route-slug}.console.log` in `$ARTIFACTS_DIR`.

5. **Evaluate.** A route passes if:
   - It loads with HTTP 200 (no 4xx/5xx).
   - No new console errors vs base.
   - Touched interactive elements respond (don't 404, don't throw uncaught exceptions).

## Output

End with the verdict line. List passed/failed routes inline.

```
Routes tested:
  /settings    PASS    artifacts/browser/settings.png
  /users/123   FAIL    console error: ReferenceError: foo

VERDICT: FAIL: 1 of 2 routes failed (see above)
```

PASS / FAIL / ESCALATE — pick exactly one for the trailing VERDICT line.
