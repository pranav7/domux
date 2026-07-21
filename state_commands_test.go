package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestClearTmuxWindowNameRenamesThenReenablesAutomaticRename(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
display-message) echo "zsh" ;;
esac
exit 0
`, callFile)

	if err := clearTmuxWindowName("mysession", "2"); err != nil {
		t.Fatalf("clearTmuxWindowName: %v", err)
	}

	data, err := os.ReadFile(callFile)
	if err != nil {
		t.Fatalf("read call file: %v", err)
	}
	calls := string(data)
	for _, want := range []string{
		"display-message -t mysession:2 -p #{automatic-rename-format}",
		"rename-window -t mysession:2 zsh",
		"set-window-option -t mysession:2 automatic-rename on",
		"refresh-client -S",
	} {
		if !strings.Contains(calls, want) {
			t.Fatalf("calls missing %q; calls=%q", want, calls)
		}
	}
}

func TestClearTmuxWindowNameNoWindowTargetsSessionOnly(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
display-message) echo "bash" ;;
esac
exit 0
`, callFile)

	if err := clearTmuxWindowName("mysession", ""); err != nil {
		t.Fatalf("clearTmuxWindowName: %v", err)
	}

	data, err := os.ReadFile(callFile)
	if err != nil {
		t.Fatalf("read call file: %v", err)
	}
	if !strings.Contains(string(data), "rename-window -t mysession bash") {
		t.Fatalf("calls=%q, want rename-window targeting mysession without window suffix", data)
	}
}

func TestClearTmuxWindowNamePropagatesDisplayMessageError(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
display-message) exit 1 ;;
esac
exit 0
`, callFile)

	if err := clearTmuxWindowName("mysession", "1"); err == nil {
		t.Fatalf("clearTmuxWindowName: want error when display-message fails")
	}
}

func TestClearWindowNameCommandUsesFlagsOverCurrentSession(t *testing.T) {
	callFile := filepath.Join(t.TempDir(), "tmux-call")
	installFakeTmux(t, `#!/bin/sh
printf '%s\n' "$*" >> "$DOMUX_TMUX_CALL"
case "$1" in
display-message) echo "fish" ;;
esac
exit 0
`, callFile)

	if err := clearWindowNameCommand([]string{"--session", "other", "--window", "3"}); err != nil {
		t.Fatalf("clearWindowNameCommand: %v", err)
	}

	data, err := os.ReadFile(callFile)
	if err != nil {
		t.Fatalf("read call file: %v", err)
	}
	if !strings.Contains(string(data), "rename-window -t other:3 fish") {
		t.Fatalf("calls=%q, want rename-window targeting other:3", data)
	}
}

func TestClearWindowNameCommandRejectsExtraArgs(t *testing.T) {
	if err := clearWindowNameCommand([]string{"extra"}); err == nil {
		t.Fatalf("clearWindowNameCommand: want error on unexpected positional arg")
	}
}
