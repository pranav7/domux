# Repository Guidelines

## Project Structure & Module Organization

`domux` is a small Go CLI/TUI in one root package. Core entrypoints and command
handlers live in `main.go`, `commands.go`, and `state_commands.go`. Session,
store, resolver, install, picker, and TUI behavior are split into matching
`*.go` files at the repo root. Tests live beside code as `*_test.go`. Runtime
state is external: TODO files under `~/.local/share/domux/`, generated
integrations under `~/.config/domux/`. The built `domux` binary and `dist/` are
ignored.

## Build, Test, and Development Commands

- `go test ./...` runs all unit tests.
- `go build ./...` checks every package builds.
- `go run .` starts the TODO TUI for the current domux context.
- `go run . sessions` opens the session picker.
- `go run . install tmux` previews generated tmux integration.
- `go run . install tmux --apply` writes integration files; review preview first.
- `gofmt -w *.go` formats root Go files before commit.

## Coding Style & Naming Conventions

Use standard Go style: tabs from `gofmt`, short package-local names, simple data
structures, and explicit error returns. Keep this repo self-contained and avoid
unneeded abstraction. Test names use `TestBehaviorCondition`, matching existing
examples like `TestPickerEscapeQuitsAfterStartup`.

## Testing Guidelines

Use Go's built-in `testing` package. Add focused tests beside the file under
change, especially for session state, install output, picker behavior, and store
parsing. Prefer table tests only when cases share the same setup. Run
`go test ./...` before handing off.

## Commit & Pull Request Guidelines

Recent commits use concise imperative subjects, for example `Clear stale PR state
with session state`. Keep subjects short, no trailing period. Do not commit
directly to restricted branches (`main`, `master`, `workspace-*`) unless
explicitly told.

PRs should include a short problem/solution summary, tests run, and screenshots
or terminal output for visible TUI or integration changes. Link issues when
relevant.

## Agent-Specific Notes

Be concise. Do not revert user edits. Follow Rob Pike's rules: measure before
optimizing, keep algorithms simple, and let data shape the code.
