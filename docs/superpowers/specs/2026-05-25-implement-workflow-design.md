---
title: Implement Workflow Pipeline
date: 2026-05-25
status: draft
owner: pranav
---

# Implement Workflow Pipeline

A hands-off implementation pipeline shipped as a **Claude Code plugin** hosted from the `domux` repository. Wraps the existing brainstorm → spec → plan workflow with a post-implementation gauntlet (simplify via the official `code-simplifier` plugin, lint auto-fix, browser test, UX review, external code review by `azcodex`) and hands off to the existing `commit-push-pr` + `babysit` exit.

## Goals

- **One command, full pipeline.** `/implement` picks up a spec / plan / Linear issue / free-text task, runs the full chain, opens a PR, babysits it.
- **Stages are individually invokable.** `/browser-test`, `/ux-review`, `/azcodex-review` each work standalone on the current diff. Useful when you want one specific gate without re-running everything.
- **Lean on official plugins.** Anthropic's `code-simplifier`, `frontend-design`, `typescript-lsp`, and `pyright-lsp` plugins are declared as dependencies. Auto-installed when our plugin installs; auto-updated by Claude Code.
- **Ships through domux.** The plugin lives at `plugins/implement-pipeline/` inside the domux repository, registered as a marketplace via `domux install claude --apply`.

## Non-goals

- Replacing the brainstorming, writing-plans, or subagent-driven-development skills from superpowers. The pipeline composes them.
- Auto-creating git worktrees. Users `cd` into a worktree first if they want isolation.
- A `/simplify` or `/lint-fix` slash command we own. `code-simplifier` is invoked by the orchestrator directly; lint auto-fix is an inline Bash one-liner; LSP plugins surface residual errors automatically post-edit.
- Hooks-based enforcement. Possible follow-up via `update-config`; out of scope here.

## Architecture

### Shape

**Orchestrator skill + bespoke specialist skills + official-plugin dependencies.** Validated against the 2026 Claude Code consensus pattern (main agent owns planning + integration; specialists own bounded tasks with their own context). The orchestrator sequences gates and handles fail-loops; specialists execute one stage each on the current diff.

### Packaging — Claude plugin via domux marketplace

domux's repository doubles as a Claude plugin marketplace. The marketplace catalog at `domux/.claude-plugin/marketplace.json` declares one plugin (`implement-pipeline`) that lives at `domux/plugins/implement-pipeline/`.

When `domux install claude --apply` runs, it shells out to `claude plugin marketplace add` + `claude plugin install` to register the marketplace and pull the plugin. Auto-update at session start keeps it fresh thereafter.

**Trade-off vs. embedding assets in the Go binary:** plugin packaging gives us free auto-updates, declarative dependency on official plugins, and standard `/plugin` UX. Cost: users need `claude` CLI installed (already true for anyone using Claude Code). domux's install code shrinks rather than grows.

### File layout (in domux repo)

```
domux/
  .claude-plugin/
    marketplace.json                        ← marketplace catalog (new)
  plugins/
    implement-pipeline/
      .claude-plugin/
        plugin.json                         ← plugin manifest
      commands/
        implement.md                        ← orchestrator entry
        browser-test.md                     ← standalone
        ux-review.md                        ← standalone
        azcodex-review.md                   ← standalone
      skills/
        implement-workflow/
          SKILL.md                          ← orchestrator skill
          simplify-prompt.md                ← prompts for dispatched subagents
          lint-prompt.md
          browser-test-prompt.md
          ux-review-prompt.md
          azcodex-prompt.md
        browser-test/SKILL.md
        ux-review/SKILL.md
        azcodex-review/SKILL.md
```

No standalone `simplify` or `lint-fix` skills or commands — see "What we don't build" below.

### marketplace.json

```json
{
  "name": "domux",
  "owner": { "name": "pranav7", "url": "https://github.com/pranav7" },
  "allowCrossMarketplaceDependenciesOn": ["claude-plugins-official"],
  "plugins": [
    {
      "name": "implement-pipeline",
      "source": "./plugins/implement-pipeline",
      "description": "Hands-off implementation pipeline: simplify + lint + browser test + UX review + external code review."
    }
  ]
}
```

`allowCrossMarketplaceDependenciesOn` is required because our plugin depends on plugins from the official marketplace.

### plugin.json

```json
{
  "name": "implement-pipeline",
  "version": "0.1.0",
  "description": "Hands-off implementation pipeline orchestrating simplify, lint, browser test, UX review, and azcodex external review.",
  "author": { "name": "pranav7" },
  "homepage": "https://github.com/pranav7/domux",
  "dependencies": [
    { "name": "code-simplifier",  "marketplace": "claude-plugins-official" },
    { "name": "frontend-design",  "marketplace": "claude-plugins-official" },
    { "name": "typescript-lsp",   "marketplace": "claude-plugins-official" },
    { "name": "pyright-lsp",      "marketplace": "claude-plugins-official" }
  ]
}
```

Unversioned dependencies — track upstream latest, auto-updated by Claude Code. We can pin versions later if a breaking change bites.

### What official plugins give us "for free"

- `code-simplifier` registers a `code-simplifier:code-simplifier` agent. The orchestrator dispatches it for stage 2 via the Agent tool.
- `frontend-design` registers a `frontend-design:frontend-design` skill our `ux-review` skill invokes.
- `typescript-lsp` + `pyright-lsp` give Claude automatic type/lint diagnostics after every edit. This shrinks our lint stage to a one-line auto-fix (`ruff --fix`/`prettier --write`/etc.); residual issues surface in-context via LSP.

## Pipeline stages

| # | Stage | Source | Skip when |
|---|---|---|---|
| 1 | Implement | `superpowers:subagent-driven-development` (user-installed) | Never |
| 2 | Simplify | Dispatch `code-simplifier:code-simplifier` agent on the diff | No source files changed |
| 3 | Lint auto-fix | Orchestrator Bash one-liner with autodetection (see below) | No `.py`/`.ts`/`.tsx`/`.js`/`.jsx` changed |
| 4 | Browser test | `implement-pipeline:browser-test` skill | No UI files changed |
| 5 | UX + scope review | `implement-pipeline:ux-review` skill (invokes `frontend-design:frontend-design` internally) | Same UI heuristic |
| 6 | azcodex external review | Bash call: `azcodex review --base <main> "..."` | Never |
| 7 | Address azcodex findings | Subagent dispatch with findings + diff | No findings |
| 8 | PR + babysit | Existing `/commit-push-pr` + `/loop 5m /babysit` | `--no-pr` flag |

### Stage order rationale

- **Simplify before lint:** simplifier may introduce style violations the linter catches.
- **Browser test before UX review:** reviewer needs to see a working feature, not just code.
- **UX before azcodex:** UX is internal judgment; azcodex is external second-opinion. External goes last on the polished artifact.
- **Stage 7 loops back into stage 3 + 4 only** before re-running stage 6. Max 2 iterations before escalating.

### Lint stage detail

Orchestrator runs detection once at stage start:

```
if exists pyproject.toml + diff touches .py:
    ruff check --fix . && ruff format . (or black if [tool.black] present)
if exists biome.json + diff touches .ts|.tsx|.js|.jsx:
    biome check --apply .
elif exists package.json + diff touches .ts|.tsx|.js|.jsx:
    npx eslint --fix <changed-files> && npx prettier --write <changed-files>
```

LSP plugins surface anything residual to Claude in-context — no separate "report residual errors" step needed in our pipeline.

### Skip rules

Run once at orchestrator start from `git diff --name-only <base>...HEAD`:

- Python lint: any `*.py` changed
- JS/TS lint: any `*.ts`/`*.tsx`/`*.js`/`*.jsx` changed
- UI heuristic (browser test + UX review): any `*.tsx`/`*.jsx`/`*.html`/`*.css` OR path under `pages/`, `app/`, `routes/`, `components/`

## Gate contracts

### Common interface

Each gate ends its output with a verdict line so the orchestrator can parse mechanically:

```
VERDICT: PASS
VERDICT: FAIL: <one-line reason>
VERDICT: ESCALATE: <one-line reason>
```

### Per-gate spec

**Simplify (dispatch `code-simplifier:code-simplifier` agent)**
- Input: "Simplify recently changed files (diff vs `<base>`). Preserve all functionality. Commit your changes."
- Output: agent's response; orchestrator treats any returned text as PASS unless errors detected.

**Lint auto-fix (inline Bash)**
- Run detected toolchain; if anything was auto-fixed, commit `chore: lint auto-fix`; re-run lint; non-zero exit → `VERDICT: FAIL` with stderr.

**Browser test (`implement-pipeline:browser-test` skill)**
- Input: changed UI route inferences from diff; login-wall protocol.
- Does: starts dev server if not running; opens browser via `browse` skill (existing on Pranav's system); exercises each touched route's primary flow; captures screenshots + console + network logs to `.claude-pipeline/artifacts/browser/`. Records GIF on failure.
- Login wall:
  1. Try `.env.test` creds.
  2. On fail: Slack-DM `<@U0ADJBVPGUC>` in `#all_` with localhost URL + "click login, react ✅".
  3. Background agent polls thread + channel: 1m, 3m, 9m backoff.
  4. Resume when ✅ seen. After 13min, escalate.
- Output: `VERDICT` + artifact paths.

**UX + scope review (`implement-pipeline:ux-review` skill)**
- Input: spec/plan path (if any) + diff + browser screenshots from stage 4.
- Does: invokes `frontend-design:frontend-design` for design-system / sizing / spacing / alignment evaluation. **Also** scope discipline: every UI element traces to a spec requirement; no dead buttons; no orphaned routes.
- Findings classified P0 (blocker) / P1 (must-fix) / P2 (nice).
- Output: `VERDICT: PASS` if zero P0/P1; else `VERDICT: FAIL` + findings list.

**azcodex external review (`implement-pipeline:azcodex-review` skill / Bash call)**
- Input: base branch (default `main`), spec/plan path if exists.
- Does: `azcodex review --base <main> "<templated-prompt>" > .claude-pipeline/azcodex.md`. Prompt focuses on correctness, security, regressions; spec/plan attached for compliance check.
- Parses for severity markers: `[CRITICAL]`, `[BLOCKING]`, `[NIT]`.
- Output: `VERDICT: PASS` if zero critical/blocking; else `VERDICT: FAIL` + count + path.

**Address azcodex findings**
- Fresh `general-purpose` subagent receives findings file + diff; addresses each; commits.
- Re-runs stages 3 + 4 + 6. Max 2 iterations before escalate.

## Run state

### State file

`.claude-pipeline/state.json` (gitignored, repo root):

```json
{
  "run_id": "2026-05-25-1730",
  "input": "docs/superpowers/specs/2026-05-25-foo-design.md",
  "input_kind": "spec",
  "base": "main",
  "stages": {
    "implement": {"status": "completed", "commit": "abc123", "attempts": 1},
    "simplify":  {"status": "completed", "commit": "def456", "attempts": 1},
    "lint":      {"status": "completed", "commit": null,     "attempts": 1},
    "browser":   {"status": "in_progress", "attempts": 1},
    "ux":        {"status": "pending"},
    "azcodex":   {"status": "pending"},
    "pr":        {"status": "pending"}
  },
  "started_at": "2026-05-25T17:30:00Z"
}
```

Standalone stage commands (e.g. `/browser-test` alone) write the same file so `/implement --resume` skips completed stages.

### Artifacts directory

`.claude-pipeline/` (gitignored):

```
.claude-pipeline/
  state.json
  azcodex.md
  artifacts/browser/{*.png,*.gif,console.log}
  findings/ux.md
```

Orchestrator adds `.claude-pipeline/` to `.gitignore` on first run if missing.

## Entry-point detection

```
/implement                          → resume in-progress run, else newest unfinished plan, else ask
/implement <path>                   → classify by frontmatter+path: spec → writing-plans → execute
                                                                       plan → execute
/implement "free text"              → small task; skip plan-writing; one-task informal plan
/implement LIN-1234                 → defer to /fix-linear-issue; then run pipeline
/implement #42                      → gh issue view 42 → treat body as informal task
/implement --resume                 → continue from state.json
/implement --from <stage>           → re-run from a specific gate
/implement --skip <stage,…>         → skip specific gates
/implement --no-pr                  → stop after stage 7
```

Worktree default: runs in current branch. Users `cd` into a worktree first if they want isolation.

## What we don't build

Explicitly NOT shipped as part of this plugin:

| Capability | Why not | Use instead |
|---|---|---|
| `/simplify` command | `code-simplifier` plugin's agent is invoked by the orchestrator directly | Dispatch `code-simplifier:code-simplifier` agent yourself if you want standalone |
| `simplify-and-dedup` skill | Same — `code-simplifier` already exists and is official | `code-simplifier:code-simplifier` |
| `/lint-fix` command | One-liner; LSP catches residual; not worth a wrapper | Run `ruff --fix .` / `biome check --apply .` directly |
| `lint-fix` skill | Same | — |
| Final code-review pass on the PR | Two-stage review inside `subagent-driven-development` already covers this per-task; azcodex covers the final-state external sanity check | Existing reviews suffice |

## domux integration

### Source-tree changes

New top-level dirs in domux repo: `.claude-plugin/` (marketplace catalog) and `plugins/implement-pipeline/` (the plugin itself). No new Go embeds — plugin files are read by Claude Code directly, not the domux binary.

### `domux install claude` changes (`install.go`)

Existing behavior preserved (patch `~/.claude/settings.json`, write `start-task.md`). New behavior added at the end:

1. Detect whether `claude` CLI is on `$PATH`. If not, print a warning and skip plugin steps.
2. Determine plugin source:
   - If `domux` binary is running from a clone of the repo (heuristic: `os.Executable()` walks up to find `.git` and `.claude-plugin/marketplace.json`), use local path: `claude plugin marketplace add /local/path/to/domux`.
   - Otherwise use GitHub: `claude plugin marketplace add pranav7/domux`.
3. Run: `claude plugin install implement-pipeline@domux`.

Preview mode (`--apply` omitted): print the shell commands that would run. Apply mode: execute them with output piped through.

### Tests (`install_test.go`)

- `TestInstallClaudePreviewMentionsPluginCommands` — preview output contains the `claude plugin marketplace add` and `claude plugin install` strings.
- `TestInstallClaudeSkipsPluginStepsWhenClaudeCliMissing` — without `claude` on PATH, install completes with a warning, no shell-out attempted.
- `TestPluginManifestIsValid` — `plugins/implement-pipeline/.claude-plugin/plugin.json` parses; required fields present; dependency entries well-formed.
- `TestMarketplaceManifestIsValid` — `.claude-plugin/marketplace.json` parses; plugin entry resolves to existing directory; `allowCrossMarketplaceDependenciesOn` includes `claude-plugins-official`.
- `TestStartTaskStillInstalled` — regression: existing start-task.md path/content still written.

We do **not** test by actually running `claude plugin install` in CI — too much state. Tests validate manifests + install command construction; full end-to-end is a one-off `make smoke-install` for developers.

## Rollout

### Versioning

Plugin uses git tags of the form `implement-pipeline--v<semver>` per Claude's convention. Tagging is optional for v0.1.0 — untagged installs track head of main, fine for early iteration. Tag once we want to pin versions.

### Migration from existing dotfiles commands

Pranav has `~/dotfiles/claude/commands/browser-test.md` (a one-liner). After this lands, the dotfiles copy should be deleted to avoid shadowing the plugin command. One-time manual cleanup; doc in PR description.

The other dotfiles commands (`babysit.md`, `commit-push-pr.md`, `check-pr-comments.md`) stay — the pipeline calls them by name. Future work could fold them into the plugin.

### Release

Ship in a minor version of domux (e.g. `v0.2.0`). Existing `.claude/commands/release.md` flow handles the version bump. README updated with `install claude --apply` mentioning the plugin.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| `azcodex` output format changes → parser breaks | Parse failure → `VERDICT: ESCALATE`, never silently pass. |
| Browser test login-wall stalls | 13min total timeout → escalate. |
| Simplify ↔ lint ping-pong | Max 2 iterations per gate; on excess, escalate. |
| User runs `/implement` mid-feature without committing | Orchestrator refuses if `git status` not clean and no `--resume`. |
| `claude plugin install` requires CLI on PATH | `domux install claude` warns if missing; doesn't break existing install flow. |
| Cross-marketplace dependency blocked | `allowCrossMarketplaceDependenciesOn` in marketplace.json handles this explicitly. |
| Official plugin breaking change (e.g. code-simplifier API) | Auto-update brings it. If we hit pain, pin via versioned dependency. |
| domux not yet published as a Claude marketplace | Init commit needs both `.claude-plugin/marketplace.json` and one tagged plugin commit before users can install. |

## Open questions deferred to implementation plan

- Whether to register domux's marketplace by GitHub repo (`pranav7/domux`) or by raw URL — repo is simpler.
- Exact heuristic for detecting "binary is running from a clone" vs "binary is in `~/.local/bin`" in install.go.
- Format of UX review findings file (markdown vs JSON for downstream consumption).
- Whether the address-findings subagent should escalate to a stronger model for security-tagged azcodex findings.
