package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

// withTempCaffeinatePID swaps caffeinatePIDFile to a path under t.TempDir for
// the duration of the test and restores it on cleanup. Returns the temp path.
func withTempCaffeinatePID(t *testing.T) string {
	t.Helper()
	orig := caffeinatePIDFile
	p := filepath.Join(t.TempDir(), "domux-caffeinate.pid")
	caffeinatePIDFile = p
	t.Cleanup(func() { caffeinatePIDFile = orig })
	return p
}

// Regression: before the fix caffeinateRunning fell back to `pgrep -x
// caffeinate`, so any unrelated caffeinate process (Claude Code's
// `caffeinate -i -t 300`, the user's own tmux binding, etc.) flipped the
// picker to "on". With the fix, no PID file → off, period.
func TestCaffeinateRunningIgnoresExternalCaffeinate(t *testing.T) {
	withTempCaffeinatePID(t)

	// Spawn an unrelated caffeinate so pgrep -x caffeinate would match.
	cmd := exec.Command("caffeinate", "-t", "5")
	if err := cmd.Start(); err != nil {
		t.Skipf("caffeinate not available: %v", err)
	}
	defer func() {
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
	}()

	if caffeinateRunning() {
		t.Fatalf("caffeinateRunning must be false when domux did not start it, even if other caffeinate processes exist")
	}
}

func TestCaffeinateRunningFalseWhenNoPIDFile(t *testing.T) {
	withTempCaffeinatePID(t)
	if caffeinateRunning() {
		t.Fatalf("caffeinateRunning must be false when no PID file exists")
	}
}

func TestOurCaffeinateAliveClearsStalePIDFile(t *testing.T) {
	path := withTempCaffeinatePID(t)
	// PID 1 (launchd) is alive but is not caffeinate.
	if err := os.WriteFile(path, []byte("1\n"), 0644); err != nil {
		t.Fatalf("write pid file: %v", err)
	}
	if ourCaffeinateAlive() {
		t.Fatalf("ourCaffeinateAlive must reject a PID whose comm is not caffeinate")
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("stale PID file should be removed, stat err=%v", err)
	}
}

func TestOurCaffeinateAliveClearsDeadPIDFile(t *testing.T) {
	path := withTempCaffeinatePID(t)
	// Spawn a short-lived child and reap it so its PID is dead-stale.
	c := exec.Command("/bin/sh", "-c", "exit 0")
	if err := c.Run(); err != nil {
		t.Fatalf("spawn: %v", err)
	}
	if err := os.WriteFile(path, []byte(strconv.Itoa(c.Process.Pid)+"\n"), 0644); err != nil {
		t.Fatalf("write pid file: %v", err)
	}
	if ourCaffeinateAlive() {
		t.Fatalf("ourCaffeinateAlive must return false for a dead PID")
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("dead-PID file should be cleaned up, stat err=%v", err)
	}
}

func TestCaffeinateStatusLabel(t *testing.T) {
	withTempCaffeinatePID(t)
	got := caffeinateStatusLabel()
	if !strings.HasPrefix(got, "caffeinate: ") {
		t.Fatalf("expected prefix 'caffeinate: ', got %q", got)
	}
	want := "caffeinate: off"
	if !caffeinateSupported() {
		want = "caffeinate: unsupported"
	}
	if got != want {
		t.Fatalf("expected %q when no PID file, got %q", want, got)
	}
}

func TestCaffeinateCommandUnknownSub(t *testing.T) {
	if err := caffeinateCommand([]string{"banana"}); err == nil {
		t.Fatalf("unknown subcommand should return error")
	}
}
