package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDebugLogDestDisabledByDefault(t *testing.T) {
	t.Setenv("DOMUX_DEBUG", "")
	if got := debugLogDest(); got != "" {
		t.Fatalf("dest = %q, want empty when unset", got)
	}
	for _, off := range []string{"0", "false", "off", "no", "OFF"} {
		t.Setenv("DOMUX_DEBUG", off)
		if got := debugLogDest(); got != "" {
			t.Fatalf("dest = %q for %q, want empty", got, off)
		}
	}
}

func TestDebugLogDestUsesExplicitPath(t *testing.T) {
	want := filepath.Join(t.TempDir(), "trace.log")
	t.Setenv("DOMUX_DEBUG", want)
	if got := debugLogDest(); got != want {
		t.Fatalf("dest = %q, want %q", got, want)
	}
}

func TestDebugLogDestFallsBackToDataDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("DOMUX_DEBUG", "1")
	want := filepath.Join(home, ".local", "share", "domux", "debug.log")
	if got := debugLogDest(); got != want {
		t.Fatalf("dest = %q, want %q", got, want)
	}
}

func TestDebugLogWritesWhenEnabled(t *testing.T) {
	dest := filepath.Join(t.TempDir(), "trace.log")
	t.Setenv("DOMUX_DEBUG", dest)

	debugLog("hello %d", 42)
	debugLog("world")

	data, err := os.ReadFile(dest)
	if err != nil {
		t.Fatalf("reading log: %v", err)
	}
	got := string(data)
	if !strings.Contains(got, "hello 42") || !strings.Contains(got, "world") {
		t.Fatalf("log missing entries:\n%s", got)
	}
	if lines := strings.Count(strings.TrimSpace(got), "\n") + 1; lines != 2 {
		t.Fatalf("got %d lines, want 2:\n%s", lines, got)
	}
}

func TestDebugLogNoOpWhenDisabled(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("DOMUX_DEBUG", "")
	t.Setenv("HOME", dir) // ensure no default file gets created either

	debugLog("should not be written")

	// Nothing should have been created anywhere under the data dir.
	def := filepath.Join(dir, ".local", "share", "domux", "debug.log")
	if _, err := os.Stat(def); !os.IsNotExist(err) {
		t.Fatalf("debug.log should not exist when disabled (stat err = %v)", err)
	}
}
