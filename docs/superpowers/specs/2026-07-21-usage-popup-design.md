# domux usage popup — design

**Date:** 2026-07-21
**Status:** Approved (design), pending spec review
**Scope:** v1 = subscription usage bars only (see Non-goals for the deferred breakdown panel)

## Problem

`domux` has two popups today — the session switcher (`prefix + s`) and the utilities
list (`prefix + u`). We want a third: a bound-to-a-shortcut popup that shows Claude
subscription usage at a glance, mirroring what Claude Code's `/usage` command displays:

```
Current session          15% used   Resets 8:29pm
Current week (all models) 24% used   Resets Jul 27 at 8:59pm
Current week (Fable)       4% used   Resets Jul 27 at 8:59pm
```

This is the "am I about to hit my limit?" glance, available from anywhere in tmux
without opening a Claude session.

## Non-goals (v1)

Claude Code's `/usage` screen has a second layer — "What's contributing to your usage"
plus skill / subagent / plugin / MCP percentage tables. Those are **not** served by the
usage endpoint; Claude Code computes them from local telemetry. Replicating them means a
separate local-log analysis engine whose numbers can't be verified against Claude's real
figures. **Deferred to a possible phase 2.** v1 renders the three subscription bars only.

## Data source

Decision: **call Claude's authenticated usage endpoint; on any failure show an honest
"unavailable" state — never fabricate numbers** (recommendation confirmed with an
independent review pass from AZCodex / GPT-5.6).

### Why not local-log estimation

A local estimate (parsing `~/.claude` session JSONL, ccusage-style) needs **no network
and no credentials**, but it structurally cannot reproduce what the bars mean:
server-enforced rolling windows, usage from *other devices*, model attribution, and
plan-side adjustments all live server-side. A local guess would look authoritative while
being wrong — worse than showing nothing. So local estimation is rejected even as a
fallback: the fallback is an honest empty state.

### The endpoint (confirmed from the Claude Code binary)

Reverse-reading the Claude Code binary (`~/.local/share/claude/versions/2.1.216`, a
bun-compiled bundle) confirmed the real mechanism:

- **Call:** `fetchUtilization: GET /api/oauth/usage` — the exact call behind the bars.
- **Host:** `https://api.anthropic.com` (base host used for oauth API calls).
- **Auth:** OAuth Bearer token + an `anthropic-beta` header. There is also an
  `anthropic-ratelimit-unified-*` response-header family (status/reset/overage/etc.).
- **Credential storage (macOS):** Claude Code reads/writes its OAuth credentials through
  the **`security` CLI** (`find-generic-password` / `add-generic-password` /
  `delete-generic-password`), *not* a plaintext file. This is ideal for domux — it matches
  the repo's "shell out with `exec.Command`, no third-party libraries" convention, so **no
  Keychain Go dependency is needed.**

### Risk assessment

Reusing the locally-stored Claude OAuth token to call **Anthropic's own** authenticated
endpoint is not materially riskier than Claude Code itself doing so, provided domux:

- reads the token **only on demand** (when the popup opens), never eagerly;
- sends it **only** to `api.anthropic.com` over HTTPS;
- **never** persists, logs, or displays the token;
- makes the **minimum** request needed (one GET, short timeout, no retry storm).

The real risk is **contractual / maintenance fragility**: the endpoint is undocumented and
can change or reject the token at any time. This is contained by isolating all of it in one
provider module (below), so an endpoint/header/schema change is a one-file fix.

### Open implementation-time constants

Three exact literals are **not yet pinned** and are deliberately isolated as named
constants in the provider. They are discoverable read-only (`strings <binary> | grep …`);
resolving them is an implementation step, not a design unknown. (Note: the auto-mode
classifier repeatedly — and correctly — blocked automated credential-store reconnaissance
during design; these will be resolved at implementation via a one-time permission grant or
by the user running the read-only grep on their own machine.)

1. **Keychain service + account** literals passed to `security find-generic-password`
   (`-s <service>` / `-a <account>`).
2. **Usage response JSON field names** — the keys `GET /api/oauth/usage` returns (the
   utilization values + reset timestamps for the three windows).
3. The exact **`anthropic-beta`** header value used on oauth requests.

If any of these is wrong at runtime, the failure surfaces as the honest "unavailable"
state, not a crash or a fabricated number.

> **Status (as-built, 2026-07-21):** Implemented on `main`. The constants remain
> best-guess in `usage_source.go` with `CONFIRM-AT-VERIFY` markers — **still to be pinned
> against one live response.** Env overrides ship for verifying without pinning:
> `DOMUX_USAGE_FIXTURE=<path>` (render a captured JSON with no network) and
> `DOMUX_CLAUDE_TOKEN=<tok>` (supply the token directly, bypassing the Keychain).
> Hardening learned in review: a window present but missing its `utilization` field is now
> **skipped, not rendered as 0%** (partial schema drift must not fabricate a number). When
> pinning the real field names, keep that invariant — prefer pointer fields + skip-on-nil
> over defaulting to zero.

## Architecture

Three isolated units, mirroring the existing `picker.go` + `pr_cache.go` + palette-in-`tui.go`
split.

| Unit | File | Responsibility | Depends on |
|---|---|---|---|
| **Usage provider** | `usage_source.go` | Read token via `security`, call `GET /api/oauth/usage`, parse + normalize into `UsageSnapshot`. Knows nothing about rendering. | `os/exec`, `net/http`, `encoding/json` |
| **Usage TUI** | `usage.go` | bubbletea `usageModel` (Init/Update/View). Renders bars in the switcher/Catppuccin style. Knows nothing about HTTP or keychain. | provider, palette + helpers from `tui.go`/`picker.go` |
| **Wiring** | `commands.go`, `main.go`, `install.go` | Register `usage` subcommand, help line, tmux bind-key. | — |

The **provider is the single fragility-containment boundary.** The TUI depends only on the
normalized `UsageSnapshot`, so schema/endpoint drift never reaches rendering.

`net/http` would be the **first HTTP client in the repo** (today domux makes zero network
calls; its only external I/O is `exec` to `tmux` and `gh`). That's expected and acceptable
for this feature.

### Data flow

```
popup opens
  → usageModel.Init fires a fetch tea.Cmd (runs off the render thread, like pickerPRRefreshCmd)
    → provider.Fetch(ctx):
        1. token := readKeychainToken()          // security find-generic-password -w
        2. GET https://api.anthropic.com/api/oauth/usage
              Authorization: Bearer <token>
              anthropic-beta: <value>
           (context timeout ~5s, single attempt)
        3. parse JSON → UsageSnapshot             // decoupled from raw shape
  → Update stores snapshot or error
  → View renders three bars, or the "unavailable" state
```

## Data contract

Deliberately decoupled from the raw endpoint JSON so schema drift stays in the provider:

```go
type UsageWindow struct {
    Label    string    // "Current session" | "Current week (all models)" | "Current week (Fable)"
    Percent  int       // 0–100, clamped
    ResetsAt time.Time // rendered in the machine's local tz
}

type UsageSnapshot struct {
    Windows   []UsageWindow
    FetchedAt time.Time
}
```

Provider surface (small interface so the TUI and tests can inject a fake — no network or
keychain in tests):

```go
type UsageProvider interface {
    Fetch(ctx context.Context) (UsageSnapshot, error)
}
```

## Rendering & UX

Compact popup matching the switcher/utilities look — Catppuccin Mocha palette (the vars in
`tui.go`), the domux wordmark header, `fitANSI`/`padLines` helpers from `picker.go`.

**Bar style — adopt the statusline meter** (`~/dotfiles/claude/statusline-command.sh`), not
chunky blocks: a thin 20-cell meter using `━` (filled) and `╌` (empty), colored by pressure
threshold — the whole bar green below 70%, amber (yellow) 70–89%, red at 90%+. This is the
exact green→amber→red behavior the user asked to match. Colors come from the existing
palette vars (`green`/`yellow`/`red`).

```
  Current session
  ━━━╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌  15% used   Resets 8:29pm

  Current week (all models)
  ━━━━━╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌  24% used   Resets Jul 27 at 8:59pm

  Current week (Fable)          ← the word "Fable" rendered in crimson
  ━╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌   4% used   Resets Jul 27 at 8:59pm

  r refresh · esc close
```

- Reset times rendered in the machine's **local timezone**.
- **"Fable" always renders crimson.** Wherever the word Fable appears (v1: the "Current
  week (Fable)" window label), render it in a crimson accent — a dedicated
  `fableCrimson` color constant (e.g. `#DC143C`, tuned against the Mocha palette during
  impl), distinct from the palette's pinkish `red` used for the ≥90% bar-pressure color.
  The bar's own color still follows the green→amber→red pressure thresholds; only the
  *label word* is crimson.
- Keys: `r` re-fetches; `esc` / `q` / `ctrl+c` closes. (Bubbletea alt-screen, like the picker.)
- Popup size: small, like the utilities popup (~`-w 60% -h 60%`, tuned during impl).

### States

- **Loading:** `Fetching usage…` with the existing picker spinner frames.
- **Failure (any):** a single honest line — `Usage unavailable` — plus a short *safe*
  reason: `no credentials found`, `auth rejected — re-login in Claude`, `network timeout`,
  or `unexpected response`. **Never** fabricated numbers. `r` retries.
- The token value is never logged, cached to disk, or rendered.

## Command & tmux wiring

- `commands.go`: add `case "usage": return usageCommand(args)` to the `runCommand` switch.
  `usageCommand` takes no positional args (like `sessions`); if it ever needs flags, use the
  `flag.NewFlagSet("usage", flag.ContinueOnError)` + `fs.SetOutput(os.Stderr)` pattern.
- `main.go`: add a help line to `printUsage`.
- `install.go` `generatedTmuxConfig()`: add a bind-key, e.g.
  `bind-key U display-popup -E -w 60% -h 60% "$HOME/bin/domux usage"`
  (`u` is already `commands`; `U` is free — final key chosen during impl). Preview-first like
  the rest of `install`, applied with `--apply`.

## Testing

Matches the repo's table-driven `_test.go` style. No network or keychain touched in tests.

- **Parser:** raw JSON fixture bytes → `UsageSnapshot`. Cover the happy path and each error
  mapping (non-200, malformed JSON, missing fields → error, not panic).
- **Bar renderer:** pure `renderBar(percent, width) string` — boundaries (0 / 69 / 70 / 89 /
  90 / 100) and the three color thresholds.
- **TUI:** inject a fake `UsageProvider` returning a fixed snapshot / a fixed error; assert
  the View shows bars vs. the "unavailable" line. (Follows how existing picker tests drive
  the model.)

## Phasing

- **v1 (this spec):** three subscription bars via `/api/oauth/usage`, honest failure state,
  tmux bind-key, tests.
- **Phase 2 (deferred, not committed):** the "what's contributing" breakdown panel from
  local telemetry. Captured here so it's on record; explicitly out of scope for v1.
