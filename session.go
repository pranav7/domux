package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

const timeFormat = time.RFC3339

type SessionState struct {
	Name           string            `json:"name"`
	Root           string            `json:"root,omitempty"`
	TodoPath       string            `json:"todo_path,omitempty"`
	FocusedTodoID  string            `json:"focused_todo_id,omitempty"`
	Label          string            `json:"label,omitempty"`
	Server         bool              `json:"server,omitempty"`
	Workspace      string            `json:"workspace,omitempty"`
	AI             map[string]string `json:"ai,omitempty"`
	AIWorkingLabel string            `json:"ai_working_label,omitempty"`
	CreatedAt      string            `json:"created_at,omitempty"`
	UpdatedAt      string            `json:"updated_at,omitempty"`
}

type AIStates struct {
	Claude       string
	Codex        string
	WorkingLabel string
}

type DomuxContext struct {
	Session  string
	Root     string
	TodoPath string
	State    *SessionState
	Source   string
}

func domuxDataDir(parts ...string) (string, error) {
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("cannot get home directory: %w", err)
	}
	all := append([]string{homeDir, ".local", "share", "domux"}, parts...)
	return filepath.Join(all...), nil
}

func domuxConfigDir(parts ...string) (string, error) {
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("cannot get home directory: %w", err)
	}
	all := append([]string{homeDir, ".config", "domux"}, parts...)
	return filepath.Join(all...), nil
}

func sessionStatePath(session string) (string, error) {
	dir, err := domuxDataDir("sessions")
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, sanitizeSessionName(session)+".json"), nil
}

func removeSessionState(session string) error {
	path, err := sessionStatePath(session)
	if err != nil {
		return err
	}
	if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("cannot remove %s: %w", path, err)
	}
	return nil
}

func sanitizeSessionName(name string) string {
	name = strings.ReplaceAll(name, "/", "%2F")
	name = strings.ReplaceAll(name, ":", "%3A")
	return name
}

func loadSessionState(session string) (*SessionState, error) {
	path, err := sessionStatePath(session)
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var state SessionState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, fmt.Errorf("cannot parse %s: %w", path, err)
	}
	if state.Name == "" {
		state.Name = session
	}
	if state.AI == nil {
		state.AI = map[string]string{}
	}
	mergeLegacyState(&state)
	return &state, nil
}

func loadSessionStateWithLegacy(session string) *SessionState {
	state, err := loadSessionState(session)
	if err == nil {
		return state
	}
	state = &SessionState{Name: session, AI: map[string]string{}}
	mergeLegacyState(state)
	return state
}

func saveSessionState(state *SessionState) error {
	if state == nil || state.Name == "" {
		return fmt.Errorf("session state needs a name")
	}
	now := time.Now().Format(timeFormat)
	if state.CreatedAt == "" {
		state.CreatedAt = now
	}
	state.UpdatedAt = now
	// The "<agent>:legacy" pseudo-pane is a synthetic key produced by
	// mergeFreshLegacyAIStateFile from the raw ~/.tmux-<agent>-<session>
	// file. It must never be persisted — otherwise a stale CLAUDING value
	// from a crashed Claude session gets baked into the JSON and outlives
	// the legacy file it came from.
	if state.AI != nil {
		delete(state.AI, "claude:legacy")
		delete(state.AI, "codex:legacy")
	}
	if !stateHasWorkingAIState(state) {
		state.AIWorkingLabel = ""
	}
	if state.AI != nil && len(state.AI) == 0 {
		state.AI = nil
	}

	path, err := sessionStatePath(state.Name)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return fmt.Errorf("cannot create session state dir: %w", err)
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("cannot encode session state: %w", err)
	}
	data = append(data, '\n')
	tmp := fmt.Sprintf("%s.%d.%d.tmp", path, os.Getpid(), time.Now().UnixNano())
	if err := os.WriteFile(tmp, data, 0644); err != nil {
		return fmt.Errorf("cannot write %s: %w", tmp, err)
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("cannot rename %s: %w", tmp, err)
	}
	return nil
}

func listSessionStates() ([]SessionState, error) {
	dir, err := domuxDataDir("sessions")
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(dir)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("cannot read %s: %w", dir, err)
	}
	var states []SessionState
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, entry.Name()))
		if err != nil {
			continue
		}
		var state SessionState
		if json.Unmarshal(data, &state) == nil && state.Name != "" {
			mergeLegacyState(&state)
			states = append(states, state)
		}
	}
	return states, nil
}

func mergeLegacyState(state *SessionState) {
	homeDir, err := os.UserHomeDir()
	if err != nil || state == nil || state.Name == "" {
		return
	}

	if state.Label == "" {
		if data, err := os.ReadFile(filepath.Join(homeDir, ".tmux-label-"+state.Name)); err == nil {
			state.Label = strings.TrimSpace(string(data))
		}
	}
	if !state.Server {
		if data, err := os.ReadFile(filepath.Join(homeDir, ".tmux-server-"+state.Name)); err == nil {
			state.Server = strings.TrimSpace(string(data)) == "running"
		}
	}
	if state.Workspace == "" {
		if data, err := os.ReadFile(filepath.Join(homeDir, ".tmux-workspace-"+state.Name)); err == nil {
			state.Workspace = strings.TrimSpace(string(data))
		}
	}
	if state.AI == nil {
		state.AI = map[string]string{}
	}
	pruneStaleAIStates(state, homeDir, "claude")
	pruneStaleAIStates(state, homeDir, "codex")
	mergeLegacyAIStateFiles(state, homeDir, "claude")
	mergeLegacyAIStateFiles(state, homeDir, "codex")
	mergeFreshLegacyAIStateFile(state, homeDir, "claude")
	mergeFreshLegacyAIStateFile(state, homeDir, "codex")
}

// pruneStaleAIStates drops JSON AI entries whose backing per-pane legacy file
// is stale. setAIState writes both atomically, so the legacy file's mtime is
// a reliable freshness signal for the JSON entry. An entry with a stale
// legacy file is an orphan from a crashed/killed agent (Stop never fired).
// Missing legacy file → trust the JSON (covers code paths that write state
// directly without going through setAIState).
func pruneStaleAIStates(state *SessionState, homeDir, agent string) {
	prefix := strings.ToLower(agent) + ":"
	for key, value := range state.AI {
		if !strings.HasPrefix(key, prefix) {
			continue
		}
		pane := strings.TrimPrefix(key, prefix)
		if pane == "legacy" {
			// transient pseudo-pane from mergeFreshLegacyAIStateFile;
			// saveSessionState strips it before write.
			continue
		}
		if normalizeAIState(agent, value) == "" {
			continue
		}
		legacyPath := filepath.Join(homeDir, ".tmux-"+agent+"-"+state.Name+"_"+pane)
		info, err := os.Stat(legacyPath)
		if err != nil {
			continue
		}
		if legacyAIStateIsStale(value, info.ModTime()) {
			delete(state.AI, key)
		}
	}
}

func mergeLegacyAIStateFiles(state *SessionState, homeDir, agent string) {
	pattern := filepath.Join(homeDir, ".tmux-"+agent+"-"+state.Name+"_*")
	matches, _ := filepath.Glob(pattern)
	for _, path := range matches {
		info, err := os.Stat(path)
		if err != nil {
			continue
		}
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		value := strings.TrimSpace(string(data))
		if legacyAIStateIsStale(value, info.ModTime()) {
			continue
		}
		key := strings.TrimPrefix(filepath.Base(path), ".tmux-"+agent+"-"+state.Name+"_")
		state.AI[aiStateKey(agent, key)] = value
	}
}

func mergeFreshLegacyAIStateFile(state *SessionState, homeDir, agent string) {
	legacyPath := filepath.Join(homeDir, ".tmux-"+agent+"-"+state.Name)
	info, err := os.Stat(legacyPath)
	if err != nil {
		return
	}
	data, err := os.ReadFile(legacyPath)
	if err != nil {
		return
	}
	value := strings.TrimSpace(string(data))
	if legacyAIStateIsStale(value, info.ModTime()) {
		return
	}
	state.AI[aiStateKey(agent, "legacy")] = value
}

// legacyAIStateIsStale decides whether an unbacked legacy state file should be
// believed. Pre/PostToolUse hooks touch the working-state file on every tool
// call, so a CLAUDING/CODEXING value with an mtime older than ~30s means the
// agent died without firing Stop. WAITING blocks on the user and shouldn't
// expire from mtime alone — give it a generous ceiling. COMPACTING is bracketed
// by PreCompact/PostCompact but compaction itself can take a while, so give it
// a couple minutes before treating it as a crash residue.
func legacyAIStateIsStale(value string, mtime time.Time) bool {
	age := time.Since(mtime)
	switch normalizeAIStateValue(value) {
	case "CLAUDING", "CODEXING":
		return age > 30*time.Second
	case "COMPACTING":
		return age > 5*time.Minute
	case "WAITING", "CLAUDE WAITING", "CODEX WAITING":
		return age > 30*time.Minute
	default:
		return true
	}
}

func resolveDomuxContext(cwd string) (*DomuxContext, error) {
	session, sessionErr := currentTmuxSession()
	if sessionErr == nil {
		return resolveDomuxContextForSession(session, cwd)
	}

	root, err := resolveRoot(cwd)
	if err != nil {
		return nil, err
	}
	path, err := resolvePath(root)
	if err != nil {
		return nil, err
	}
	return &DomuxContext{Root: root, TodoPath: path, Source: "path"}, nil
}

func resolveDomuxContextForSession(session, fallbackDir string) (*DomuxContext, error) {
	state := loadSessionStateWithLegacy(session)
	if state.Root != "" {
		todoPath := state.TodoPath
		if todoPath == "" {
			var err error
			todoPath, err = resolvePath(state.Root)
			if err != nil {
				return nil, err
			}
		}
		return &DomuxContext{
			Session:  session,
			Root:     state.Root,
			TodoPath: todoPath,
			State:    state,
			Source:   "session",
		}, nil
	}

	root, err := resolveRoot(fallbackDir)
	if err != nil {
		return nil, err
	}
	path, err := resolvePath(root)
	if err != nil {
		return nil, err
	}
	return &DomuxContext{
		Session:  session,
		Root:     root,
		TodoPath: path,
		State:    state,
		Source:   "path",
	}, nil
}

func setSessionRoot(session, root string) (*SessionState, error) {
	todoPath, err := resolvePath(root)
	if err != nil {
		return nil, err
	}
	state := loadSessionStateWithLegacy(session)
	state.Name = session
	state.Root = root
	state.TodoPath = todoPath
	if state.AI == nil {
		state.AI = map[string]string{}
	}
	if err := saveSessionState(state); err != nil {
		return nil, err
	}
	return state, nil
}

func focusedOrTopItem(list *List, state *SessionState) (Item, bool) {
	if list == nil || len(list.Active) == 0 {
		return Item{}, false
	}
	if state != nil && state.FocusedTodoID != "" {
		for _, item := range list.Active {
			if item.ID == state.FocusedTodoID {
				return item, true
			}
		}
	}
	return list.Active[0], true
}

func aggregateAIStateFromSession(state *SessionState) string {
	states := aggregateAIStatesFromSession(state)
	if states.Claude == "WAITING" || states.Codex == "WAITING" {
		return "WAITING"
	}
	if states.Claude != "" {
		return states.Claude
	}
	return states.Codex
}

func aggregateAIStatesFromSession(state *SessionState) AIStates {
	var states AIStates
	if state == nil {
		return states
	}
	for key, value := range state.AI {
		agent := inferAgentFromAIKey(key)
		if agent == "" {
			agent = inferAgentFromAIValue(value)
		}
		if agent == "" {
			agent = "claude"
		}
		value = normalizeAIState(agent, value)
		switch agent {
		case "codex":
			states.Codex = mergeAIState(states.Codex, value)
		default:
			states.Claude = mergeAIState(states.Claude, value)
		}
	}
	if states.Claude == "CLAUDING" || states.Codex == "CODEXING" {
		states.WorkingLabel = state.AIWorkingLabel
		if states.WorkingLabel == "" {
			states.WorkingLabel = fallbackAIWorkingLabel(state)
		}
	}
	return states
}

func stateHasWorkingAIState(state *SessionState) bool {
	if state == nil {
		return false
	}
	for key, value := range state.AI {
		agent := inferAgentFromAIKey(key)
		if agent == "" {
			agent = inferAgentFromAIValue(value)
		}
		if agent == "" {
			agent = "claude"
		}
		if normalizeAIState(agent, value) == workingAIState(agent) {
			return true
		}
	}
	return false
}

func fallbackAIWorkingLabel(state *SessionState) string {
	parts := []string{state.Name}
	for key, value := range state.AI {
		agent := inferAgentFromAIKey(key)
		if agent == "" {
			agent = inferAgentFromAIValue(value)
		}
		if agent == "" {
			agent = "claude"
		}
		if normalizeAIState(agent, value) == workingAIState(agent) {
			parts = append(parts, key+"="+value)
		}
	}
	sort.Strings(parts[1:])
	return stableAIWorkingLabel(strings.Join(parts, "\x00"))
}

func mergeAIState(current, next string) string {
	if aiStateRank(next) > aiStateRank(current) {
		return next
	}
	return current
}

// aiStateRank — precedence for collapsing per-pane states down to one per
// agent. WAITING (user-blocking) outranks COMPACTING (transient, agent-busy)
// outranks the plain working states.
func aiStateRank(value string) int {
	switch value {
	case "WAITING":
		return 3
	case "COMPACTING":
		return 2
	case "CLAUDING", "CODEXING":
		return 1
	default:
		return 0
	}
}

func inferAgentFromAIKey(key string) string {
	agent, _, ok := strings.Cut(key, ":")
	if !ok {
		return ""
	}
	switch strings.ToLower(agent) {
	case "claude", "codex":
		return strings.ToLower(agent)
	default:
		return ""
	}
}

func inferAgentFromAIValue(value string) string {
	switch normalizeAIStateValue(value) {
	case "CODEXING", "CODEX WAITING":
		return "codex"
	case "CLAUDING", "CLAUDE WAITING":
		return "claude"
	default:
		// COMPACTING is agent-agnostic — let the caller decide (defaults to
		// claude since PreCompact is a Claude Code-only hook today).
		return ""
	}
}

func normalizeAIState(agent, value string) string {
	value = normalizeAIStateValue(value)
	switch value {
	case "", "IDLE", "CLEAR":
		return ""
	case "CLAUDE WAITING", "CODEX WAITING", "WAITING":
		return "WAITING"
	case "COMPACTING":
		return "COMPACTING"
	case "CODEXING":
		if agent == "codex" {
			return "CODEXING"
		}
	case "CLAUDING":
		if agent == "claude" {
			return "CLAUDING"
		}
	}
	return ""
}

func normalizeAIStateValue(value string) string {
	value = strings.ToUpper(strings.TrimSpace(value))
	value = strings.ReplaceAll(value, "_", " ")
	return strings.Join(strings.Fields(value), " ")
}

func aiStateKey(agent, pane string) string {
	if pane == "" {
		pane = "default"
	}
	return strings.ToLower(agent) + ":" + pane
}
