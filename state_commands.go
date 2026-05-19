package main

import (
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

func serverCommand(args []string) error {
	fs := flag.NewFlagSet("server", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	sessionFlag := fs.String("session", "", "tmux session name")
	if err := fs.Parse(args); err != nil {
		return err
	}
	mode := "toggle"
	if fs.NArg() > 1 {
		return fmt.Errorf("server accepts at most one mode")
	}
	if fs.NArg() == 1 {
		mode = fs.Arg(0)
	}
	session := *sessionFlag
	if session == "" {
		var err error
		session, err = currentTmuxSession()
		if err != nil {
			return err
		}
	}

	state := loadSessionStateWithLegacy(session)
	switch mode {
	case "on", "set", "running":
		return setServerSessionByName(session)
	case "off", "clear":
		state.Server = false
	case "toggle":
		if state.Server {
			state.Server = false
		} else {
			return setServerSessionByName(session)
		}
	default:
		return fmt.Errorf("unknown server mode %q", mode)
	}
	if err := saveSessionState(state); err != nil {
		return err
	}
	homeDir, _ := os.UserHomeDir()
	if homeDir != "" {
		_ = removeHomeFile(homeDir, ".tmux-server-"+session)
	}
	return refreshTmuxClient()
}

func labelCommand(args []string) error {
	fs := flag.NewFlagSet("label", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	sessionFlag := fs.String("session", "", "tmux session name")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() < 1 {
		return fmt.Errorf("label requires set VALUE or clear")
	}
	session := *sessionFlag
	if session == "" {
		var err error
		session, err = currentTmuxSession()
		if err != nil {
			return err
		}
	}
	state := loadSessionStateWithLegacy(session)
	homeDir, _ := os.UserHomeDir()

	switch fs.Arg(0) {
	case "set":
		if fs.NArg() < 2 {
			return fmt.Errorf("label set requires a value")
		}
		state.Label = strings.Join(fs.Args()[1:], " ")
		if homeDir != "" {
			if err := writeHomeFile(homeDir, ".tmux-label-"+session, state.Label+"\n"); err != nil {
				return err
			}
		}
	case "clear":
		state.Label = ""
		if homeDir != "" {
			if err := removeHomeFile(homeDir, ".tmux-label-"+session); err != nil {
				return err
			}
		}
	default:
		return fmt.Errorf("unknown label command %q", fs.Arg(0))
	}
	if err := saveSessionState(state); err != nil {
		return err
	}
	return refreshTmuxClient()
}

func aiStateCommand(args []string) error {
	fs := flag.NewFlagSet("ai-state", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	sessionFlag := fs.String("session", "", "tmux session name")
	paneFlag := fs.String("pane", "", "pane key")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("ai-state requires CLAUDING, WAITING, IDLE, clear, or toggle")
	}
	session := *sessionFlag
	if session == "" {
		var err error
		session, err = currentTmuxSession()
		if err != nil {
			return err
		}
	}
	pane := *paneFlag
	if pane == "" {
		pane = currentTmuxPaneKey()
	}
	state := strings.ToUpper(fs.Arg(0))
	if state == "CLEAR" {
		state = ""
	}
	return setAIState(session, pane, state)
}

func workspaceCommand(args []string) error {
	fs := flag.NewFlagSet("workspace", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	sessionFlag := fs.String("session", "", "tmux session name")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("workspace requires occupied or free")
	}
	value := fs.Arg(0)
	if value != "occupied" && value != "free" {
		return fmt.Errorf("workspace state must be occupied or free")
	}
	session := *sessionFlag
	if session == "" {
		var err error
		session, err = currentTmuxSession()
		if err != nil {
			return err
		}
	}
	state := loadSessionStateWithLegacy(session)
	state.Workspace = value
	if err := saveSessionState(state); err != nil {
		return err
	}
	homeDir, _ := os.UserHomeDir()
	if homeDir != "" {
		return writeHomeFile(homeDir, ".tmux-workspace-"+session, value+"\n")
	}
	return nil
}

func currentTmuxPaneKey() string {
	out, err := execOutput("tmux", "display-message", "-p", "#{window_index}_#{pane_index}")
	if err != nil || strings.TrimSpace(out) == "" {
		return "default"
	}
	return strings.TrimSpace(out)
}

func setAIState(session, pane, value string) error {
	if pane == "" {
		pane = "default"
	}
	state := loadSessionStateWithLegacy(session)
	if state.AI == nil {
		state.AI = map[string]string{}
	}
	current := strings.TrimSpace(state.AI[pane])
	if value == "TOGGLE" {
		switch current {
		case "CLAUDING":
			value = "WAITING"
		case "WAITING":
			value = ""
		default:
			value = "CLAUDING"
		}
	}
	if value == "" || value == "IDLE" {
		delete(state.AI, pane)
	} else {
		state.AI[pane] = value
	}
	if err := saveSessionState(state); err != nil {
		return err
	}

	homeDir, _ := os.UserHomeDir()
	if homeDir != "" {
		name := ".tmux-claude-" + session + "_" + pane
		if value == "" || value == "IDLE" {
			if err := removeHomeFile(homeDir, name); err != nil {
				return err
			}
		} else if err := writeHomeFile(homeDir, name, value+"\n"); err != nil {
			return err
		}
	}
	return refreshTmuxClient()
}

func clearWaitingState(session string) {
	state := loadSessionStateWithLegacy(session)
	changed := false
	for pane, value := range state.AI {
		if strings.TrimSpace(value) == "WAITING" {
			state.AI[pane] = "IDLE"
			changed = true
		}
	}
	if changed {
		_ = saveSessionState(state)
	}

	homeDir, _ := os.UserHomeDir()
	if homeDir == "" {
		return
	}
	pattern := filepath.Join(homeDir, ".tmux-claude-"+session+"_*")
	matches, _ := filepath.Glob(pattern)
	for _, path := range matches {
		data, err := os.ReadFile(path)
		if err == nil && strings.TrimSpace(string(data)) == "WAITING" {
			_ = os.WriteFile(path, []byte("IDLE\n"), 0644)
		}
	}
	legacyFile := filepath.Join(homeDir, ".tmux-claude-"+session)
	data, err := os.ReadFile(legacyFile)
	if err == nil && strings.TrimSpace(string(data)) == "WAITING" {
		_ = os.WriteFile(legacyFile, []byte("IDLE\n"), 0644)
	}
}

func execOutput(name string, args ...string) (string, error) {
	out, err := exec.Command(name, args...).Output()
	return strings.TrimSpace(string(out)), err
}
