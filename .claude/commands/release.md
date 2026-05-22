---
description: Cut a domux release — tag HEAD, push, watch the GH Action.
argument-hint: <version> (e.g. v0.2.0)
---

Cut a new domux release at version `$ARGUMENTS`.

The release flow for this repo is:
- Tag pushed to GitHub → `.github/workflows/release.yml` builds darwin arm64 + amd64 binaries → publishes a GitHub release with tarballs + `SHA256SUMS`.

Do the following in order. Stop and report if any step fails — do not retry destructively.

## 1. Validate the version

- `$ARGUMENTS` must look like `vMAJOR.MINOR.PATCH` (e.g. `v0.1.0`). Reject anything else.
- Run `git tag --list "$ARGUMENTS"` — if the tag already exists, stop and tell the user.

## 2. Verify clean release state

- `git rev-parse --abbrev-ref HEAD` must equal `main`. If not, stop.
- `git fetch origin main` then verify `git rev-parse HEAD == git rev-parse origin/main`. If local is ahead, ask the user to push first. If behind, ask before fast-forwarding.
- `git status --porcelain` — warn the user if the working tree is dirty, but proceed if they confirm (in-progress unrelated work is common here).
- Confirm `go build ./...` and `go test ./...` both pass on the current HEAD. If either fails, stop.

## 3. Confirm with the user

Show a one-line summary like:

```
Release v0.2.0 at <short-sha> "<commit subject>" — push tag and trigger CI?
```

Wait for explicit confirmation before continuing.

## 4. Tag and push

- `git tag -a $ARGUMENTS -m "Release $ARGUMENTS"`
- `git push origin $ARGUMENTS`

## 5. Watch the workflow

- `gh run list --workflow=release.yml --limit 1 --json databaseId,status,conclusion,headBranch` to find the run.
- `gh run watch <run-id> --exit-status` to follow it. If it fails, print the failed job's log tail and stop — do not retry without user input.

## 6. Report the release URL

When the workflow succeeds:
- `gh release view $ARGUMENTS --json url,assets --jq '{url, assets: [.assets[].name]}'`
- Print the URL and the asset list so the user can verify the curl installer one-liner will resolve correctly.

## Notes

- Never use `--force` on the tag push.
- If the user passes no version, ask which version they want — don't auto-bump silently.
- This command does NOT merge feature branches. Run that yourself before tagging.
