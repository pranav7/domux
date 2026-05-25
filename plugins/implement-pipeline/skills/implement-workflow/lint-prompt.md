# Lint detail (orchestrator reference)

This is reference for the orchestrator, not a dispatched prompt.

Detection order:

1. Python (`.py` in $CHANGED + pyproject.toml exists):
   - If `[tool.ruff]` in pyproject.toml → `ruff check --fix . && ruff format .`
   - Else if `[tool.black]` in pyproject.toml → `ruff check --fix . && black .` (ruff still does the lint fixes)
   - Else skip Python lint.

2. JS/TS (`.ts|.tsx|.js|.jsx` in $CHANGED):
   - If `biome.json` or `biome.jsonc` → `npx --no-install biome check --apply .`
   - Else if `package.json` → `npx --no-install eslint --fix <files> && npx --no-install prettier --write <files>`
   - Else skip JS/TS lint.

If anything was auto-fixed, commit `chore: lint auto-fix`. Re-run the checker (no `--fix`) and treat non-zero as FAIL.

LSP plugins (typescript-lsp, pyright-lsp) report residual issues to Claude in-context — no separate report step needed.
