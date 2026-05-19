package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const timeFormat = time.RFC3339

type SessionState struct {
	Name          string            `json:"name"`
	Root          string            `json:"root,omitempty"`
	TodoPath      string            `json:"todo_path,omitempty"`
	FocusedTodoID string            `json:"focused_todo_id,omitempty"`
	Label         string            `json:"label,omitempty"`
	Server        bool              `json:"server,omitempty"`
	Workspace     string            `json:"workspace,omitempty"`
	AI            map[string]string `json:"ai,omitempty"`
	CreatedAt     string            `json:"created_at,omitempty"`
	UpdatedAt     string            `json:"updated_at,omitempty"`
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
	pattern := filepath.Join(homeDir, ".tmux-claude-"+state.Name+"_*")
	matches, _ := filepath.Glob(pattern)
	for _, path := range matches {
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		key := strings.TrimPrefix(filepath.Base(path), ".tmux-claude-"+state.Name+"_")
		state.AI[key] = strings.TrimSpace(string(data))
	}
	if len(state.AI) == 0 {
		legacyPath := filepath.Join(homeDir, ".tmux-claude-"+state.Name)
		if info, err := os.Stat(legacyPath); err == nil && time.Since(info.ModTime()) < 2*time.Minute {
			if data, err := os.ReadFile(legacyPath); err == nil {
				state.AI["legacy"] = strings.TrimSpace(string(data))
			}
		}
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
	if state == nil {
		return ""
	}
	hasClauding := false
	for _, value := range state.AI {
		switch strings.TrimSpace(value) {
		case "WAITING":
			return "WAITING"
		case "CLAUDING":
			hasClauding = true
		}
	}
	if hasClauding {
		return "CLAUDING"
	}
	return ""
}
