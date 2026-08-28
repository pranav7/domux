package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"time"
)

const artboardsFailureThreshold = 2
const artboardsHealthCheckTimeout = 300 * time.Millisecond // loopback round-trip, not a real network call
const artboardsLaunchAgentLabel = "dev.domux.artboards"

// artboardsHealthState is the on-disk record of the artboards server's
// recent reachability, shared by the picker's poll and `domux artboards
// status`. Unlike the usage cache (a TTL'd value cache that skips a network
// call when fresh), every check here is a fresh live probe — only the
// consecutive-failure counter persists, so one slow/dropped probe never
// flips the picker to "down" by itself; ConsecutiveFailures must reach
// artboardsFailureThreshold first.
type artboardsHealthState struct {
	LastCheckedAt       time.Time `json:"last_checked_at"`
	LastOKAt            time.Time `json:"last_ok_at,omitempty"`
	ConsecutiveFailures int       `json:"consecutive_failures"`
	LastError           string    `json:"last_error,omitempty"`
}

func artboardsHealthStatePath() (string, error) {
	return domuxDataDir("artboards-health.json")
}

func loadArtboardsHealthState() artboardsHealthState {
	var state artboardsHealthState
	path, err := artboardsHealthStatePath()
	if err != nil {
		return state
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return state
	}
	_ = json.Unmarshal(data, &state)
	return state
}

func saveArtboardsHealthState(state artboardsHealthState) error {
	path, err := artboardsHealthStatePath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", filepath.Dir(path), err)
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0644); err != nil {
		return fmt.Errorf("cannot write %s: %w", tmp, err)
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("cannot rename %s: %w", tmp, err)
	}
	return nil
}

// artboardsHealthURL is a var, not a literal, so a test can point it at an
// httptest.Server — mirroring the caffeinatePIDFile package-var swap
// convention in caffeinate_test.go.
var artboardsHealthURL = func() string {
	return fmt.Sprintf("http://127.0.0.1:%d/healthz", artboardsPort)
}

// probeArtboardsHealth performs one live GET, updates and persists the
// consecutive-failure counter, and reports whether the server should now be
// considered down. Both the picker's poll and `domux artboards status`
// (indirectly, via the shared state file) see the same debounce.
func probeArtboardsHealth(ctx context.Context) (bool, artboardsHealthState) {
	state := loadArtboardsHealthState()
	client := &http.Client{Timeout: artboardsHealthCheckTimeout}

	var ok bool
	var lastErr error
	if req, err := http.NewRequestWithContext(ctx, http.MethodGet, artboardsHealthURL(), nil); err == nil {
		if resp, ferr := client.Do(req); ferr == nil {
			ok = resp.StatusCode == http.StatusOK
			_ = resp.Body.Close()
		} else {
			lastErr = ferr
		}
	} else {
		lastErr = err
	}

	state.LastCheckedAt = time.Now()
	if ok {
		state.ConsecutiveFailures = 0
		state.LastOKAt = state.LastCheckedAt
		state.LastError = ""
	} else {
		state.ConsecutiveFailures++
		if lastErr != nil {
			state.LastError = lastErr.Error()
		}
	}
	_ = saveArtboardsHealthState(state)
	return state.ConsecutiveFailures >= artboardsFailureThreshold, state
}

func artboardsSupported() bool { return runtime.GOOS == "darwin" }

func errArtboardsUnsupported() error {
	return fmt.Errorf("artboards supervision is only supported on macOS (current OS: %s)", runtime.GOOS)
}

func artboardsPlistPath() (string, error) {
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("cannot get home directory: %w", err)
	}
	return filepath.Join(homeDir, "Library", "LaunchAgents", artboardsLaunchAgentLabel+".plist"), nil
}

func artboardsLogPath() (string, error) {
	return domuxDataDir("artboards", ".server.log")
}

// artboardsPlistContent generates a *user* LaunchAgent — unprivileged, no
// sudo, under ~/Library/LaunchAgents (unlike caffeinatePlistContent's
// /Library/LaunchDaemons, which needs root only because full-mode caffeinate
// has to call pmset as root). binPath is resolved once via os.Executable()
// at install time and baked in, not looked up on PATH when launchd starts
// the job — launchd's inherited PATH can't be trusted to find `domux`.
func artboardsPlistContent(binPath, logPath string) string {
	return fmt.Sprintf(`<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>%s</string>
    <key>ProgramArguments</key>
    <array>
        <string>%s</string>
        <string>artboards</string>
        <string>serve</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>%s</string>
    <key>StandardErrorPath</key>
    <string>%s</string>
</dict>
</plist>
`, artboardsLaunchAgentLabel, binPath, logPath, logPath)
}

func installArtboards(args []string) error {
	fs := flag.NewFlagSet("install artboards", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	apply := fs.Bool("apply", false, "write and load the LaunchAgent")
	uninstall := fs.Bool("uninstall", false, "unload and remove the LaunchAgent")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if !artboardsSupported() {
		fmt.Println(errArtboardsUnsupported().Error())
		return nil
	}
	plistPath, err := artboardsPlistPath()
	if err != nil {
		return err
	}

	if *uninstall {
		_ = exec.Command("launchctl", "unload", plistPath).Run()
		if err := os.Remove(plistPath); err != nil && !os.IsNotExist(err) {
			return fmt.Errorf("cannot remove %s: %w", plistPath, err)
		}
		fmt.Printf("removed %s\n", plistPath)
		return nil
	}

	binPath, err := os.Executable()
	if err != nil {
		return fmt.Errorf("cannot resolve domux binary path: %w", err)
	}
	logPath, err := artboardsLogPath()
	if err != nil {
		return err
	}
	content := artboardsPlistContent(binPath, logPath)

	if !*apply {
		fmt.Printf("Would write %s:\n\n%s\n", plistPath, content)
		fmt.Println("Then load it with: launchctl load " + plistPath)
		return nil
	}

	if err := os.MkdirAll(filepath.Dir(plistPath), 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", filepath.Dir(plistPath), err)
	}
	if err := os.MkdirAll(filepath.Dir(logPath), 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", filepath.Dir(logPath), err)
	}
	if err := backupIfExists(plistPath); err != nil {
		return err
	}
	if err := os.WriteFile(plistPath, []byte(content), 0644); err != nil {
		return fmt.Errorf("cannot write %s: %w", plistPath, err)
	}
	_ = exec.Command("launchctl", "unload", plistPath).Run() // ignore error — may not be loaded yet
	if err := exec.Command("launchctl", "load", plistPath).Run(); err != nil {
		return fmt.Errorf("launchctl load: %w", err)
	}
	fmt.Printf("installed and loaded %s\n", plistPath)
	fmt.Println("a `go build` elsewhere won't affect the already-running server until you run `domux artboards restart`")
	return nil
}

func artboardsRestartCommand(args []string) error {
	if len(args) != 0 {
		return fmt.Errorf("restart does not accept arguments")
	}
	if !artboardsSupported() {
		return errArtboardsUnsupported()
	}
	plistPath, err := artboardsPlistPath()
	if err != nil {
		return err
	}
	if !fileExists(plistPath) {
		return fmt.Errorf("artboards is not installed - run `domux install artboards --apply` first")
	}
	_ = exec.Command("launchctl", "unload", plistPath).Run()
	if err := exec.Command("launchctl", "load", plistPath).Run(); err != nil {
		return fmt.Errorf("launchctl load: %w", err)
	}
	fmt.Println("artboards restarted")
	return nil
}

// artboardsStatusCommand does its own one-shot live probe, bypassing the
// shared debounce state entirely — a human running `status` wants ground
// truth right now, not the picker's smoothed view. The debounce exists only
// to protect the ambient, frequently-polled header warning from one flaky
// response.
func artboardsStatusCommand(args []string) error {
	fs := flag.NewFlagSet("artboards status", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	asJSON := fs.Bool("json", false, "print as JSON")
	if err := fs.Parse(args); err != nil {
		return err
	}

	ctx, cancel := context.WithTimeout(context.Background(), artboardsHealthCheckTimeout)
	defer cancel()
	client := &http.Client{Timeout: artboardsHealthCheckTimeout}
	up := false
	if req, err := http.NewRequestWithContext(ctx, http.MethodGet, artboardsHealthURL(), nil); err == nil {
		if resp, err := client.Do(req); err == nil {
			up = resp.StatusCode == http.StatusOK
			_ = resp.Body.Close()
		}
	}

	if *asJSON {
		data, _ := json.Marshal(map[string]bool{"up": up})
		fmt.Println(string(data))
		return nil
	}
	if up {
		fmt.Println("artboards: up")
		return nil
	}
	fmt.Println("artboards: down")
	return nil
}
