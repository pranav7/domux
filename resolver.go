package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// resolvePath returns the markdown file path for the given directory.
func resolvePath(dir string) (string, error) {
	worktreeRoot, err := resolveRoot(dir)
	if err != nil {
		return "", err
	}

	// Sanitize the path (percent-encode slashes)
	sanitized := sanitizePath(worktreeRoot)

	// Build storage path
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("cannot get home directory: %w", err)
	}

	storageDir := filepath.Join(homeDir, ".local", "share", "domux", "by-path")
	if err := os.MkdirAll(storageDir, 0755); err != nil {
		return "", fmt.Errorf("cannot create storage directory: %w", err)
	}

	return filepath.Join(storageDir, sanitized+".md"), nil
}

// resolveRoot returns the stable root domux should use for a directory.
func resolveRoot(dir string) (string, error) {
	if dir == "" {
		var err error
		dir, err = os.Getwd()
		if err != nil {
			return "", fmt.Errorf("cannot get current directory: %w", err)
		}
	}

	worktreeRoot, err := getGitWorktreeRoot(dir)
	if err == nil && worktreeRoot != "" {
		return worktreeRoot, nil
	}

	abs, err := filepath.Abs(dir)
	if err != nil {
		return "", fmt.Errorf("cannot resolve %s: %w", dir, err)
	}
	return abs, nil
}

// getGitWorktreeRoot returns the git worktree root for the given directory.
func getGitWorktreeRoot(dir string) (string, error) {
	cmd := exec.Command("git", "rev-parse", "--show-toplevel")
	cmd.Dir = dir
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

// sanitizePath percent-encodes slashes in the path.
func sanitizePath(path string) string {
	return strings.ReplaceAll(path, "/", "%2F")
}

// unsanitizePath reverses percent-encoding of slashes.
func unsanitizePath(encoded string) string {
	return strings.ReplaceAll(encoded, "%2F", "/")
}
