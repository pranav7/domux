# Implement Pipeline

A Claude Code plugin shipped from the domux marketplace. Runs implement →
simplify → lint → browser test → UX review → external code review → PR.

## Install

```bash
domux install claude --apply
```

This patches `~/.claude/settings.json` (existing behavior) and then runs:

```bash
claude plugin marketplace add <local-path-or-pranav7/domux>
claude plugin install implement-pipeline@domux
```

`claude` CLI must be on PATH. Without it, the install prints the commands and
skips the plugin step — patching of settings still happens.

## Usage

```
/implement                           # resume or pick newest unfinished plan
/implement docs/superpowers/plans/foo.md
/implement "fix the broken signup button"
/implement LIN-1234
/implement #42
/implement --resume
/implement --from browser
/implement --skip ux,azcodex
/implement --no-pr
```

Standalone gates:

```
/browser-test [base-branch]
/ux-review    [spec-or-plan-path]
/azcodex-review [base-branch]
```

## Pipeline stages

| # | Stage         | Source                                        | Skip rule                              |
|---|---------------|-----------------------------------------------|----------------------------------------|
| 1 | Implement     | superpowers:subagent-driven-development       | Never                                   |
| 2 | Simplify      | code-simplifier:code-simplifier agent          | No source files in diff                 |
| 3 | Lint auto-fix | Inline Bash (ruff/biome/eslint/prettier)       | No .py/.ts/.tsx/.js/.jsx in diff        |
| 4 | Browser test  | implement-pipeline:browser-test                 | No UI files in diff                     |
| 5 | UX + scope    | implement-pipeline:ux-review                    | Same as 4                               |
| 6 | azcodex       | implement-pipeline:azcodex-review               | Never                                   |
| 7 | Address       | general-purpose subagent on findings            | No findings                             |
| 8 | PR + babysit  | /commit-push-pr + /loop 5m /babysit             | --no-pr                                 |

## Artifacts

`.claude-pipeline/` (gitignored):

```
.claude-pipeline/
  state.json
  azcodex.md
  artifacts/browser/{*.png,*.gif,console.log}
  findings/ux.md
```

## Dependencies (auto-installed)

- claude-plugins-official:code-simplifier
- claude-plugins-official:frontend-design
- claude-plugins-official:typescript-lsp
- claude-plugins-official:pyright-lsp

## Cleanup of legacy commands

After installing this plugin, delete `~/dotfiles/claude/commands/browser-test.md`
to avoid shadowing the plugin command. The other dotfiles commands
(`babysit.md`, `commit-push-pr.md`, `check-pr-comments.md`) stay — the
pipeline invokes them by name.
