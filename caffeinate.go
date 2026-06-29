package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"
)

// caffeinateSupported reports whether the keep-awake feature can actually run.
// It is macOS-only: it shells out to `caffeinate(8)` and (in full mode) drives
// launchd + pmset. On every other OS the feature is disabled and reports
// "unsupported" rather than erroring noisily from a tmux binding or popup.
func caffeinateSupported() bool {
	return runtime.GOOS == "darwin"
}

func errCaffeinateUnsupported() error {
	return fmt.Errorf("caffeinate is only supported on macOS (current OS: %s)", runtime.GOOS)
}

type caffeinateMode string

const (
	caffeinateModePartial caffeinateMode = "partial"
	caffeinateModeFull    caffeinateMode = "full"
)

const (
	caffeinatePlistPath  = "/Library/LaunchDaemons/com.domux.noclamshell.plist"
	caffeinateSudoersDst = "/etc/sudoers.d/domux-caffeinate"
	caffeinateConfigName = "caffeinate.json"
)

var caffeinatePIDFile = "/tmp/domux-caffeinate.pid"

type caffeinateConfig struct {
	Mode caffeinateMode `json:"mode"`
}

func caffeinateConfigPath() (string, error) {
	return domuxConfigDir(caffeinateConfigName)
}

func loadCaffeinateConfig() caffeinateConfig {
	var c caffeinateConfig
	if path, err := caffeinateConfigPath(); err == nil {
		if data, err := os.ReadFile(path); err == nil {
			_ = json.Unmarshal(data, &c)
		}
	}
	if c.Mode != caffeinateModeFull {
		c.Mode = caffeinateModePartial
	}
	return c
}

func saveCaffeinateConfig(c caffeinateConfig) error {
	path, err := caffeinateConfigPath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", filepath.Dir(path), err)
	}
	if err := backupIfExists(path); err != nil {
		return err
	}
	data, err := json.MarshalIndent(c, "", "  ")
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

func installCaffeinate(args []string) error {
	fs := flag.NewFlagSet("install caffeinate", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	full := fs.Bool("full", false, "install lid-close prevention (requires sudo)")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *full {
		return installCaffeinateFull()
	}
	return installCaffeinatePartial()
}

func installCaffeinatePartial() error {
	if err := saveCaffeinateConfig(caffeinateConfig{Mode: caffeinateModePartial}); err != nil {
		return err
	}
	fmt.Println("caffeinate registered (partial mode — prevents idle sleep only).")
	fmt.Println("Run `domux install caffeinate --full` for lid-close prevention (requires sudo).")
	return nil
}

func installCaffeinateFull() error {
	if !caffeinateSupported() {
		fmt.Printf("Full caffeinate mode is macOS-only; nothing to install on %s.\n", runtime.GOOS)
		return installCaffeinatePartial()
	}
	fmt.Println("Full caffeinate mode prevents lid-close sleep via a launchd daemon + pmset.")
	fmt.Printf("This will write %s and %s using sudo.\n", caffeinatePlistPath, caffeinateSudoersDst)
	if !confirm(os.Stdin, "Proceed? [y/N]: ") {
		fmt.Println("falling back to partial mode.")
		return installCaffeinatePartial()
	}
	if err := sudoInstallContent(caffeinatePlistContent(), "0644", caffeinatePlistPath); err != nil {
		return fmt.Errorf("install plist: %w", err)
	}
	if err := sudoInstallContent(caffeinateSudoersFragment(), "0440", caffeinateSudoersDst); err != nil {
		return fmt.Errorf("install sudoers fragment: %w", err)
	}
	if err := saveCaffeinateConfig(caffeinateConfig{Mode: caffeinateModeFull}); err != nil {
		return err
	}
	fmt.Println("caffeinate registered (full mode).")
	return nil
}

// sudoInstallContent writes content to a temp file then uses sudo install(1)
// to place it at dst with the given mode, owned by root:wheel. Backs up dst
// via sudo cp if it already exists.
func sudoInstallContent(content, mode, dst string) error {
	if err := sudoBackupIfExists(dst); err != nil {
		return err
	}
	tmp, err := os.CreateTemp("", "domux-install-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.WriteString(content); err != nil {
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := runVisible("sudo", "install", "-m", mode, "-o", "root", "-g", "wheel", tmp.Name(), dst); err != nil {
		return err
	}
	fmt.Printf("installed %s\n", dst)
	return nil
}

func sudoBackupIfExists(path string) error {
	if _, err := os.Stat(path); err != nil {
		// Either missing or unreadable as current user — try via sudo.
		if err := exec.Command("sudo", "-n", "test", "-e", path).Run(); err != nil {
			return nil
		}
	}
	backup := fmt.Sprintf("%s.domux-backup", path)
	return runVisible("sudo", "cp", "-p", path, backup)
}

func caffeinateSudoersFragment() string {
	user := os.Getenv("USER")
	if user == "" {
		user = os.Getenv("LOGNAME")
	}
	if user == "" {
		user = "root"
	}
	return fmt.Sprintf("%s ALL=(ALL) NOPASSWD: /usr/bin/pmset, /bin/launchctl\n", user)
}

func runSudoNonInteractive(args ...string) error {
	full := append([]string{"-n"}, args...)
	return exec.Command("sudo", full...).Run()
}

func caffeinatePlistContent() string {
	// Re-assert disablesleep every 30s so lid-close prevention survives wake
	// (macOS clears it on some sleep/wake cycles). KeepAlive restarts the loop
	// if it ever dies. The job exists only while loaded; caffeinateOff unloads
	// it and clears disablesleep, so it never outlives a "domux caffeinate off".
	return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.domux.noclamshell</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/sh</string>
        <string>-c</string>
        <string>while :; do /usr/bin/pmset -a disablesleep 1; sleep 30; done</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
`
}

func caffeinateRunning() bool {
	return ourCaffeinateAlive()
}

func ourCaffeinateAlive() bool {
	data, err := os.ReadFile(caffeinatePIDFile)
	if err != nil {
		return false
	}
	pid, err := strconv.Atoi(strings.TrimSpace(string(data)))
	if err != nil || pid <= 0 {
		_ = os.Remove(caffeinatePIDFile)
		return false
	}
	process, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	if process.Signal(syscall.Signal(0)) != nil {
		_ = os.Remove(caffeinatePIDFile)
		return false
	}
	if !pidIsCaffeinate(pid) {
		_ = os.Remove(caffeinatePIDFile)
		return false
	}
	return true
}

func pidIsCaffeinate(pid int) bool {
	out, err := exec.Command("ps", "-o", "comm=", "-p", strconv.Itoa(pid)).Output()
	if err != nil {
		return false
	}
	return strings.Contains(string(out), "caffeinate")
}

func caffeinateOn() error {
	if !caffeinateSupported() {
		return errCaffeinateUnsupported()
	}
	if caffeinateRunning() {
		return nil
	}
	cmd := exec.Command("caffeinate", "-dimsu")
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("start caffeinate: %w", err)
	}
	if err := os.WriteFile(caffeinatePIDFile, []byte(strconv.Itoa(cmd.Process.Pid)+"\n"), 0644); err != nil {
		_ = cmd.Process.Kill()
		return fmt.Errorf("write pid file: %w", err)
	}
	_ = cmd.Process.Release()
	if loadCaffeinateConfig().Mode == caffeinateModeFull {
		_ = runSudoNonInteractive("launchctl", "load", caffeinatePlistPath)
		_ = runSudoNonInteractive("pmset", "-a", "disablesleep", "1")
	}
	return nil
}

func caffeinateOff() error {
	if data, err := os.ReadFile(caffeinatePIDFile); err == nil {
		if pid, err := strconv.Atoi(strings.TrimSpace(string(data))); err == nil && pid > 0 && pidIsCaffeinate(pid) {
			if process, err := os.FindProcess(pid); err == nil {
				_ = process.Kill()
			}
		}
		_ = os.Remove(caffeinatePIDFile)
	}
	if loadCaffeinateConfig().Mode == caffeinateModeFull {
		_ = runSudoNonInteractive("launchctl", "unload", caffeinatePlistPath)
		_ = runSudoNonInteractive("pmset", "-a", "disablesleep", "0")
	}
	return nil
}

func toggleCaffeinate() error {
	if caffeinateRunning() {
		return caffeinateOff()
	}
	return caffeinateOn()
}

func caffeinateCommand(args []string) error {
	sub := "status"
	if len(args) > 0 {
		sub = args[0]
	}
	switch sub {
	case "status", "on", "off", "toggle":
		// valid subcommand
	default:
		return fmt.Errorf("usage: domux caffeinate [status|on|off|toggle]")
	}
	// On unsupported platforms every variant is a graceful no-op so the tmux
	// `K` binding / commands popup report "unsupported" instead of erroring.
	if !caffeinateSupported() {
		fmt.Println(caffeinateStatusLabel())
		return nil
	}
	switch sub {
	case "status":
		fmt.Println(caffeinateStatusLabel())
		return nil
	case "on":
		if err := caffeinateOn(); err != nil {
			return err
		}
		fmt.Println(caffeinateStatusLabel())
		_ = refreshTmuxClient()
		return nil
	case "off":
		if err := caffeinateOff(); err != nil {
			return err
		}
		fmt.Println(caffeinateStatusLabel())
		_ = refreshTmuxClient()
		return nil
	case "toggle":
		if err := toggleCaffeinate(); err != nil {
			return err
		}
		fmt.Println(caffeinateStatusLabel())
		_ = refreshTmuxClient()
		return nil
	default:
		return fmt.Errorf("usage: domux caffeinate [status|on|off|toggle]")
	}
}

func caffeinateStatusLabel() string {
	if !caffeinateSupported() {
		return "caffeinate: unsupported"
	}
	if caffeinateRunning() {
		return "caffeinate: on"
	}
	return "caffeinate: off"
}
