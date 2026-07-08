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
	switch fs.Arg(0) {
	case "set":
		if fs.NArg() < 2 {
			return fmt.Errorf("label set requires a value")
		}
		return setSessionLabel(session, strings.Join(fs.Args()[1:], " "))
	case "clear":
		return setSessionLabel(session, "")
	default:
		return fmt.Errorf("unknown label command %q", fs.Arg(0))
	}
}

func setSessionLabel(session, label string) error {
	label = strings.TrimSpace(label)
	state := loadSessionStateWithLegacy(session)
	state.Name = session
	state.Label = label

	homeDir, _ := os.UserHomeDir()
	if homeDir != "" {
		if label == "" {
			if err := removeHomeFile(homeDir, ".tmux-label-"+session); err != nil {
				return err
			}
		} else if err := writeHomeFile(homeDir, ".tmux-label-"+session, label+"\n"); err != nil {
			return err
		}
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
	agentFlag := fs.String("agent", "", "ai agent: claude, codex, or opencode")
	allFlag := fs.Bool("all", false, "apply to all panes for the agent")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("ai-state requires CLAUDING, CODEXING, CODING, COMPACTING, WAITING, IDLE, clear, or toggle")
	}
	session := *sessionFlag
	if session == "" {
		var err error
		session, err = currentTmuxSession()
		if err != nil {
			return err
		}
	}
	value := fs.Arg(0)
	agent := strings.ToLower(strings.TrimSpace(*agentFlag))
	if agent == "" {
		agent = inferAgentFromAIValue(value)
	}
	if agent == "" {
		agent = "claude"
	}
	if !isSupportedAIAgent(agent) {
		return fmt.Errorf("ai-state agent must be claude, codex, or opencode")
	}
	if *allFlag {
		if *paneFlag != "" {
			return fmt.Errorf("ai-state --all cannot be combined with --pane")
		}
		switch normalizeAIStateValue(value) {
		case "", "CLEAR", "IDLE":
			return clearAIStateForAgent(session, agent)
		default:
			return fmt.Errorf("ai-state --all only supports clear or IDLE")
		}
	}
	pane := *paneFlag
	if pane == "" {
		pane = currentTmuxPaneKey()
	}
	return setAIState(session, pane, agent, value)
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
	out, err := execOutput("tmux", tmuxDisplayArgs("#{window_index}_#{pane_index}")...)
	if err != nil || strings.TrimSpace(out) == "" {
		return "default"
	}
	return strings.TrimSpace(out)
}

func setAIState(session, pane, agent, value string) error {
	if pane == "" {
		pane = "default"
	}
	if agent == "" {
		agent = "claude"
	}
	state := loadSessionStateWithLegacy(session)
	if state.AI == nil {
		state.AI = map[string]string{}
	}
	key := aiStateKey(agent, pane)
	current := strings.TrimSpace(state.AI[key])
	if current == "" && agent == "claude" {
		current = strings.TrimSpace(state.AI[pane])
	}
	value = normalizeAIStateValue(value)
	if value == "TOGGLE" {
		switch current {
		case workingAIState(agent):
			value = "WAITING"
		case "WAITING":
			value = ""
		default:
			value = workingAIState(agent)
		}
	} else {
		value = normalizeAIState(agent, value)
	}
	if value == "" || value == "IDLE" {
		delete(state.AI, key)
		if agent == "claude" {
			delete(state.AI, pane)
		}
	} else {
		state.AI[key] = value
	}
	ensureAIWorkingLabels(state)
	if err := saveSessionState(state); err != nil {
		return err
	}

	homeDir, _ := os.UserHomeDir()
	if homeDir != "" {
		name := ".tmux-" + agent + "-" + session + "_" + pane
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

func clearAIStateForAgent(session, agent string) error {
	_, stateErr := loadSessionState(session)
	stateFileExists := stateErr == nil
	state := loadSessionStateWithLegacy(session)
	changed := false
	for key, value := range state.AI {
		keyAgent := inferAgentFromAIKey(key)
		if keyAgent == "" {
			keyAgent = inferAgentFromAIValue(value)
		}
		if keyAgent == "" && agent == "claude" {
			keyAgent = "claude"
		}
		if keyAgent == agent {
			delete(state.AI, key)
			changed = true
		}
	}
	if changed || stateFileExists {
		if err := saveSessionState(state); err != nil {
			return err
		}
	}

	homeDir, _ := os.UserHomeDir()
	if homeDir != "" {
		if err := removeHomeFile(homeDir, ".tmux-"+agent+"-"+session); err != nil {
			return err
		}
		if err := removeHomeFilesWithPrefix(homeDir, ".tmux-"+agent+"-"+session+"_"); err != nil {
			return err
		}
	}
	return refreshTmuxClient()
}

func workingAIState(agent string) string {
	if agent == "opencode" {
		return "CODING"
	}
	if agent == "codex" {
		return "CODEXING"
	}
	return "CLAUDING"
}

func isSupportedAIAgent(agent string) bool {
	switch agent {
	case "claude", "codex", "opencode":
		return true
	default:
		return false
	}
}

func clearWaitingState(session string) {
	state := loadSessionStateWithLegacy(session)
	changed := false
	for pane, value := range state.AI {
		if normalizeAIStateValue(value) == "WAITING" {
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
	for _, agent := range []string{"claude", "codex", "opencode"} {
		pattern := filepath.Join(homeDir, ".tmux-"+agent+"-"+session+"_*")
		matches, _ := filepath.Glob(pattern)
		for _, path := range matches {
			data, err := os.ReadFile(path)
			if err == nil && normalizeAIStateValue(string(data)) == "WAITING" {
				_ = os.WriteFile(path, []byte("IDLE\n"), 0644)
			}
		}
		legacyFile := filepath.Join(homeDir, ".tmux-"+agent+"-"+session)
		data, err := os.ReadFile(legacyFile)
		if err == nil && normalizeAIStateValue(string(data)) == "WAITING" {
			_ = os.WriteFile(legacyFile, []byte("IDLE\n"), 0644)
		}
	}
}

func execOutput(name string, args ...string) (string, error) {
	out, err := exec.Command(name, args...).Output()
	return strings.TrimSpace(string(out)), err
}
