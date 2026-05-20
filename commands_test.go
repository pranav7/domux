package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestClearSessionStateFilesRemovesLegacySessionMetadata(t *testing.T) {
	homeDir := t.TempDir()
	session := "audrey-app"
	files := []string{
		".tmux-label-" + session,
		".tmux-server-" + session,
		".tmux-workspace-" + session,
		".tmux-pr-" + session,
		".tmux-claude-" + session,
		".tmux-claude-" + session + "_0_0",
	}
	for _, name := range files {
		if err := os.WriteFile(filepath.Join(homeDir, name), []byte("value\n"), 0644); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}
	unrelated := ".tmux-pr-other-session"
	if err := os.WriteFile(filepath.Join(homeDir, unrelated), []byte("value\n"), 0644); err != nil {
		t.Fatalf("write unrelated file: %v", err)
	}

	if err := clearSessionStateFiles(homeDir, session); err != nil {
		t.Fatalf("clearSessionStateFiles: %v", err)
	}

	for _, name := range files {
		if _, err := os.Stat(filepath.Join(homeDir, name)); !os.IsNotExist(err) {
			t.Fatalf("expected %s to be removed, stat err = %v", name, err)
		}
	}
	if _, err := os.Stat(filepath.Join(homeDir, unrelated)); err != nil {
		t.Fatalf("expected unrelated file to remain: %v", err)
	}
}
