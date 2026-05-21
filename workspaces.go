package main

import (
	"os/exec"
	"strings"
)

// defaultBaseBranch returns the short branch name that `origin/HEAD` points
// at (e.g. "main", "master", "develop"). Falls back to "main" if origin/HEAD
// isn't set — keeps backwards compat with the prior hardcoded behaviour.
func defaultBaseBranch(root string) (string, error) {
	cmd := exec.Command("git", "-C", root, "symbolic-ref", "--short", "refs/remotes/origin/HEAD")
	out, err := cmd.Output()
	if err != nil {
		return "main", nil
	}
	ref := strings.TrimSpace(string(out))
	ref = strings.TrimPrefix(ref, "origin/")
	if ref == "" {
		return "main", nil
	}
	return ref, nil
}
