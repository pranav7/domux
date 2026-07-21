package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type prRefreshSession struct {
	Name   string
	Path   string
	Root   string
	Branch string
}

type prLookupFunc func(path, branch string) (*prInfo, bool, error)

func refreshPRCaches() error {
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot get home directory: %w", err)
	}
	sessions, err := currentPRRefreshSessions()
	if err != nil {
		return err
	}
	return refreshPRCachesForSessions(homeDir, sessions, lookupPRForBranch)
}

func currentPRRefreshSessions() ([]prRefreshSession, error) {
	names, err := currentTmuxSessions()
	if err != nil {
		return nil, err
	}
	var sessions []prRefreshSession
	for _, name := range names {
		state := loadSessionStateWithLegacy(name)
		path := state.Root
		if path == "" {
			path, _ = tmuxPanePath(name)
		}
		session := prRefreshSession{Name: name, Path: path}
		if path != "" {
			session.Branch = gitBranch(path)
			if root, err := gitWorktreeRoot(path); err == nil {
				if commonRoot, ok := workspaceRootFromPath(root); ok {
					session.Root = commonRoot
				} else {
					session.Root = root
				}
			}
			// A session sitting on the repo's default branch has no PR of its
			// own to show — `gh pr list --head main` matches any historical PR
			// that happened to be opened head=main (e.g. a same-branch PR merged
			// months ago), surfacing a stale badge that never clears because the
			// next refresh tick just looks it up again.
			if base, err := defaultBaseBranch(path); err == nil && session.Branch == base {
				session.Branch = ""
			}
		}
		sessions = append(sessions, session)
	}
	return sessions, nil
}

func refreshPRCachesForSessions(homeDir string, sessions []prRefreshSession, lookup prLookupFunc) error {
	live := map[string]bool{}
	for _, session := range sessions {
		if session.Name != "" {
			live[session.Name] = true
		}
	}
	if err := pruneDeadPRCaches(homeDir, live); err != nil {
		return err
	}
	if lookup == nil {
		lookup = lookupPRForBranch
	}

	type query struct {
		path     string
		branch   string
		sessions []string
	}
	queries := map[string]*query{}
	for _, session := range sessions {
		if session.Name == "" {
			continue
		}
		if session.Path == "" || session.Branch == "" {
			if err := removePRCache(homeDir, session.Name); err != nil {
				return err
			}
			continue
		}
		keyRoot := session.Root
		if keyRoot == "" {
			keyRoot = session.Path
		}
		key := keyRoot + "\x00" + session.Branch
		q := queries[key]
		if q == nil {
			q = &query{path: session.Path, branch: session.Branch}
			queries[key] = q
		}
		q.sessions = append(q.sessions, session.Name)
	}

	var firstErr error
	for _, q := range queries {
		pr, ok, err := lookup(q.path, q.branch)
		if err != nil {
			if firstErr == nil {
				firstErr = err
			}
			continue
		}
		for _, session := range q.sessions {
			if ok {
				if err := writePRCache(homeDir, session, pr); err != nil {
					return err
				}
			} else if err := removePRCache(homeDir, session); err != nil {
				return err
			}
		}
	}
	return firstErr
}

func lookupPRForBranch(path, branch string) (*prInfo, bool, error) {
	if !commandExists("gh") {
		return nil, false, errors.New("gh not found")
	}
	cmd := exec.Command("gh", "pr", "list", "--head", branch, "--state", "all", "--limit", "1", "--json", "number,state,title,isDraft")
	cmd.Dir = path
	out, err := cmd.CombinedOutput()
	if err != nil {
		return nil, false, fmt.Errorf("gh pr list %s: %w: %s", branch, err, strings.TrimSpace(string(out)))
	}
	return parseGHPRList(out)
}

func parseGHPRList(data []byte) (*prInfo, bool, error) {
	var prs []struct {
		Number  int    `json:"number"`
		State   string `json:"state"`
		Title   string `json:"title"`
		IsDraft bool   `json:"isDraft"`
	}
	if err := json.Unmarshal(data, &prs); err != nil {
		return nil, false, fmt.Errorf("cannot parse gh pr list: %w", err)
	}
	if len(prs) == 0 {
		return nil, false, nil
	}
	pr := prs[0]
	state := strings.ToUpper(strings.TrimSpace(pr.State))
	if pr.IsDraft && state == "OPEN" {
		state = "DRAFT"
	}
	return &prInfo{Number: pr.Number, State: state, Title: sanitizePRCacheTitle(pr.Title)}, true, nil
}

func readPRCache(homeDir, session string) (*prInfo, error) {
	data, err := os.ReadFile(prCachePath(homeDir, session))
	if err != nil {
		return nil, err
	}
	parts := strings.SplitN(strings.TrimSpace(string(data)), "::", 3)
	if len(parts) != 3 {
		return nil, fmt.Errorf("invalid PR cache for %s", session)
	}
	var number int
	if _, err := fmt.Sscanf(parts[0], "%d", &number); err != nil {
		return nil, fmt.Errorf("invalid PR number for %s: %w", session, err)
	}
	if number <= 0 {
		return nil, fmt.Errorf("invalid PR number for %s: %d", session, number)
	}
	return &prInfo{Number: number, State: parts[1], Title: parts[2]}, nil
}

func writePRCache(homeDir, session string, pr *prInfo) error {
	if pr == nil || pr.Number <= 0 {
		return removePRCache(homeDir, session)
	}
	path := prCachePath(homeDir, session)
	data := []byte(fmt.Sprintf("%d::%s::%s\n", pr.Number, pr.State, sanitizePRCacheTitle(pr.Title)))
	tmp, err := os.CreateTemp(homeDir, ".domux-pr-tmp-*")
	if err != nil {
		return fmt.Errorf("cannot create temp PR cache: %w", err)
	}
	tmpPath := tmp.Name()
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		_ = os.Remove(tmpPath)
		return fmt.Errorf("cannot write %s: %w", tmpPath, err)
	}
	if err := tmp.Close(); err != nil {
		_ = os.Remove(tmpPath)
		return fmt.Errorf("cannot close %s: %w", tmpPath, err)
	}
	if err := os.Chmod(tmpPath, 0644); err != nil {
		_ = os.Remove(tmpPath)
		return fmt.Errorf("cannot chmod %s: %w", tmpPath, err)
	}
	if err := os.Rename(tmpPath, path); err != nil {
		_ = os.Remove(tmpPath)
		return fmt.Errorf("cannot rename %s: %w", tmpPath, err)
	}
	return nil
}

func removePRCache(homeDir, session string) error {
	if err := os.Remove(prCachePath(homeDir, session)); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("cannot remove PR cache for %s: %w", session, err)
	}
	return nil
}

func pruneDeadPRCaches(homeDir string, live map[string]bool) error {
	entries, err := os.ReadDir(homeDir)
	if err != nil {
		return fmt.Errorf("cannot read %s: %w", homeDir, err)
	}
	for _, entry := range entries {
		name := entry.Name()
		if !strings.HasPrefix(name, ".tmux-pr-") {
			continue
		}
		session := strings.TrimPrefix(name, ".tmux-pr-")
		if !live[session] {
			if err := os.Remove(filepath.Join(homeDir, name)); err != nil && !os.IsNotExist(err) {
				return fmt.Errorf("cannot remove stale PR cache %s: %w", name, err)
			}
		}
	}
	return nil
}

func prCachePath(homeDir, session string) string {
	return filepath.Join(homeDir, ".tmux-pr-"+session)
}

func sanitizePRCacheTitle(title string) string {
	title = strings.ReplaceAll(title, "\n", " ")
	title = strings.ReplaceAll(title, "\r", " ")
	return strings.TrimSpace(title)
}
