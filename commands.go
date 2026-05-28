package main

import (
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

var errClearDirty = errors.New("uncommitted changes — commit or stash first")

func runCommand(name string, args []string) error {
	switch name {
	case "help":
		printUsage()
		return nil
	case "sessions", "switcher":
		if len(args) != 0 {
			return fmt.Errorf("%s does not accept arguments", name)
		}
		return runPicker()
	case "todo":
		if len(args) != 0 {
			return fmt.Errorf("todo does not accept arguments")
		}
		cwd, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("cannot get current directory: %w", err)
		}
		ctx, err := resolveDomuxContext(cwd)
		if err != nil {
			return err
		}
		return runTUI(ctx.TodoPath)
	case "start":
		return startSession(args)
	case "attach":
		return attachSessionCommand(args)
	case "adopt":
		return adoptSession(args)
	case "focus":
		return focusSessionTodo(args)
	case "status":
		return printTmuxStatus(args)
	case "list":
		return printListCommand(args)
	case "clear":
		if len(args) != 0 {
			return fmt.Errorf("clear does not accept arguments")
		}
		return clearWorkspace()
	case "reset-branch":
		if len(args) != 0 {
			return fmt.Errorf("reset-branch does not accept arguments")
		}
		return resetBranch()
	case "clear-state":
		if len(args) != 0 {
			return fmt.Errorf("clear-state does not accept arguments")
		}
		session, err := currentTmuxSession()
		if err != nil {
			return err
		}
		return clearSessionState(session)
	case "server":
		return serverCommand(args)
	case "set-server":
		if len(args) != 0 {
			return fmt.Errorf("set-server does not accept arguments")
		}
		return setServerSession()
	case "label":
		return labelCommand(args)
	case "ai-state":
		return aiStateCommand(args)
	case "workspace":
		return workspaceCommand(args)
	case "install":
		return installCommand(args)
	case "bootstrap":
		return bootstrapCommand(args)
	case "commands":
		if len(args) != 0 {
			return fmt.Errorf("commands does not accept arguments")
		}
		return runUtilities()
	case "caffeinate":
		return caffeinateCommand(args)
	case "doctor":
		return doctorCommand(args)
	case "migrate":
		return migrateCommand(args)
	case "claude-statusline":
		return claudeStatuslineCommand(args)
	default:
		return fmt.Errorf("unknown command %q", name)
	}
}

func startSession(args []string) error {
	fs := flag.NewFlagSet("start", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	name := fs.String("name", "", "tmux session name")
	if err := fs.Parse(args); err != nil {
		return err
	}

	dir := "."
	if fs.NArg() > 1 {
		return fmt.Errorf("start accepts at most one directory")
	}
	if fs.NArg() == 1 {
		dir = fs.Arg(0)
	}

	root, err := resolveRoot(dir)
	if err != nil {
		return err
	}

	if *name != "" {
		return createOrAttachSession(*name, root)
	}

	matches, err := matchingSessionsForRoot(root)
	if err != nil {
		return err
	}
	if session, ok := singleMatchingStartSession(matches); ok {
		return attachTmuxSession(session)
	}
	if len(matches) > 0 {
		return runPickerForSessionNames(matches)
	}

	sessionName := nextSessionName(filepath.Base(root))
	return createOrAttachSession(sessionName, root)
}

func singleMatchingStartSession(matches []string) (string, bool) {
	if len(matches) != 1 {
		return "", false
	}
	return matches[0], true
}

func adoptSession(args []string) error {
	fs := flag.NewFlagSet("adopt", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	if err := fs.Parse(args); err != nil {
		return err
	}
	dir := "."
	if fs.NArg() > 1 {
		return fmt.Errorf("adopt accepts at most one directory")
	}
	if fs.NArg() == 1 {
		dir = fs.Arg(0)
	}
	session, err := currentTmuxSession()
	if err != nil {
		return err
	}
	root, err := resolveRoot(dir)
	if err != nil {
		return err
	}
	state, err := setSessionRoot(session, root)
	if err != nil {
		return err
	}
	fmt.Printf("adopted %s -> %s\n", state.Name, state.Root)
	return nil
}

func attachSessionCommand(args []string) error {
	if len(args) != 1 {
		return fmt.Errorf("attach requires a session name")
	}
	return attachTmuxSession(args[0])
}

func createOrAttachSession(name, root string) error {
	if tmuxSessionExists(name) {
		state, err := loadSessionState(name)
		if err != nil && os.IsNotExist(err) {
			if _, err := setSessionRoot(name, root); err != nil {
				return err
			}
		} else if err == nil && state.Root == "" {
			if _, err := setSessionRoot(name, root); err != nil {
				return err
			}
		}
		return attachTmuxSession(name)
	}

	if err := createTmuxSession(name, root); err != nil {
		return err
	}
	if _, err := setSessionRoot(name, root); err != nil {
		return err
	}
	return attachTmuxSession(name)
}

func tmuxSessionExists(name string) bool {
	return exec.Command("tmux", "has-session", "-t", name).Run() == nil
}

func createTmuxSession(name, root string) error {
	cmd := exec.Command("tmux", "new-session", "-d", "-s", name, "-c", root)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux new-session: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func attachTmuxSession(name string) error {
	clearWaitingState(name)
	if inTmuxClientEnv() {
		cmd := exec.Command("tmux", tmuxAttachArgs(true, name)...)
		out, err := cmd.CombinedOutput()
		if err == nil {
			return nil
		}
		if !isNoCurrentClientOutput(out) {
			_, _ = os.Stderr.Write(out)
			return err
		}
	}
	cmd := exec.Command("tmux", tmuxAttachArgs(false, name)...)
	cmd.Env = withoutTmuxEnv(os.Environ())
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

func inTmuxClientEnv() bool {
	return strings.TrimSpace(os.Getenv("TMUX")) != ""
}

func withoutTmuxEnv(env []string) []string {
	next := make([]string, 0, len(env))
	for _, item := range env {
		if strings.HasPrefix(item, "TMUX=") || strings.HasPrefix(item, "TMUX_PANE=") {
			continue
		}
		next = append(next, item)
	}
	return next
}

func tmuxAttachArgs(inTmux bool, name string) []string {
	if inTmux {
		return []string{"switch-client", "-t", name}
	}
	return []string{"attach-session", "-t", name}
}

func matchingSessionsForRoot(root string) ([]string, error) {
	var matches []string
	seen := map[string]bool{}
	states, err := listSessionStates()
	if err != nil {
		return nil, err
	}
	for _, state := range states {
		if state.Root == root && tmuxSessionExists(state.Name) {
			matches = append(matches, state.Name)
			seen[state.Name] = true
		}
	}

	out, err := exec.Command("tmux", "list-sessions", "-F", "#{session_name}").Output()
	if err == nil {
		for _, session := range strings.Split(strings.TrimSpace(string(out)), "\n") {
			if session == "" || seen[session] {
				continue
			}
			pathOut, err := exec.Command("tmux", "display-message", "-t", session, "-p", "#{pane_current_path}").Output()
			if err != nil {
				continue
			}
			sessionRoot, err := resolveRoot(strings.TrimSpace(string(pathOut)))
			if err == nil && sessionRoot == root {
				matches = append(matches, session)
				seen[session] = true
			}
		}
	}
	sort.Strings(matches)
	return matches, nil
}

func nextSessionName(base string) string {
	base = cleanSessionName(base)
	if base == "" {
		base = "domux"
	}
	if !tmuxSessionExists(base) {
		return base
	}
	for i := 2; ; i++ {
		candidate := fmt.Sprintf("%s-%d", base, i)
		if !tmuxSessionExists(candidate) {
			return candidate
		}
	}
}

func cleanSessionName(name string) string {
	name = strings.ToLower(name)
	var b strings.Builder
	for _, r := range name {
		switch {
		case r >= 'a' && r <= 'z':
			b.WriteRune(r)
		case r >= '0' && r <= '9':
			b.WriteRune(r)
		case r == '-' || r == '_':
			b.WriteRune(r)
		case r == '.' || r == ' ':
			b.WriteRune('-')
		}
	}
	return strings.Trim(b.String(), "-")
}

func focusSessionTodo(args []string) error {
	if len(args) > 1 {
		return fmt.Errorf("focus accepts zero args or one todo id")
	}
	session, err := currentTmuxSession()
	if err != nil {
		return err
	}
	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("cannot get current directory: %w", err)
	}
	ctx, err := resolveDomuxContextForSession(session, cwd)
	if err != nil {
		return err
	}
	list, err := loadList(ctx.TodoPath)
	if err != nil {
		return err
	}
	if len(list.Active) == 0 {
		return fmt.Errorf("no active todos to focus")
	}
	id := ""
	if len(args) == 1 {
		id = args[0]
	} else {
		ensureItemID(&list.Active[0])
		id = list.Active[0].ID
		if err := saveList(ctx.TodoPath, list); err != nil {
			return err
		}
	}
	state := ctx.State
	if state == nil {
		state = loadSessionStateWithLegacy(session)
	}
	state.Name = session
	state.Root = ctx.Root
	state.TodoPath = ctx.TodoPath
	state.FocusedTodoID = id
	if err := saveSessionState(state); err != nil {
		return err
	}
	fmt.Printf("focused %s\n", id)
	return nil
}

func printListCommand(args []string) error {
	if len(args) > 0 {
		return fmt.Errorf("list does not accept arguments")
	}
	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("cannot get current directory: %w", err)
	}
	ctx, err := resolveDomuxContext(cwd)
	if err != nil {
		return err
	}
	list, err := loadList(ctx.TodoPath)
	if err != nil {
		return err
	}
	for i, item := range list.Active {
		prefix := "├─"
		if i == len(list.Active)-1 {
			prefix = "└─"
		}
		symbol := "○"
		if item.InProgress {
			symbol = "●"
		}
		fmt.Printf("%s %s %s\n", prefix, symbol, item.Title)
	}
	return nil
}

func printTmuxStatus(args []string) error {
	if len(args) > 2 {
		return fmt.Errorf("status accepts optional session and pane path")
	}
	session := ""
	if len(args) >= 1 {
		session = args[0]
	}
	cwd := ""
	if len(args) >= 2 {
		cwd = args[1]
	}
	if cwd == "" {
		var err error
		cwd, err = os.Getwd()
		if err != nil {
			return fmt.Errorf("cannot get current directory: %w", err)
		}
	}
	var ctx *DomuxContext
	var err error
	if session != "" {
		ctx, err = resolveDomuxContextForSession(session, cwd)
	} else {
		ctx, err = resolveDomuxContext(cwd)
		session = ctx.Session
	}
	if err != nil {
		return err
	}
	list, _ := loadList(ctx.TodoPath)
	status := ""
	if item, ok := focusedOrTopItem(list, ctx.State); ok {
		title := item.Title
		if len(title) > 50 {
			title = title[:47] + "..."
		}
		symbol := "○"
		if item.InProgress {
			symbol = "●"
		}
		status = fmt.Sprintf("#[default]#[fg=#f9e2af]%s %s ", symbol, title)
	}
	if ctx.State == nil && session != "" {
		ctx.State = loadSessionStateWithLegacy(session)
	}
	aiStates := aggregateAIStatesFromSession(ctx.State)
	status += tmuxAIBadge("claude", aiStates.Claude)
	status += tmuxAIBadge("codex", aiStates.Codex)
	if ctx.State != nil && ctx.State.Server {
		status += "#[default]#[fg=#f9e2af,bold] ⚡"
	}
	fmt.Print(status)
	return nil
}

func tmuxAIBadge(agent, state string) string {
	switch {
	case agent == "claude" && state == "WAITING":
		return "#[default]#[fg=#f38ba8]#[bg=#f38ba8,fg=#1e1e2e,bold] CLAUDE WAITING #[default]#[fg=#f38ba8]#[default]"
	case agent == "codex" && state == "WAITING":
		return "#[default]#[fg=#f38ba8]#[bg=#f38ba8,fg=#1e1e2e,bold] CODEX WAITING #[default]#[fg=#f38ba8]#[default]"
	case agent == "claude" && state == "COMPACTING":
		return "#[default]#[fg=#AFAFFF,bold] ✦ Compacting… ✦ #[default]"
	case agent == "codex" && state == "COMPACTING":
		return "#[default]#[fg=#AFAFFF,bold] ✦ Compacting… ✦ #[default]"
	default:
		return ""
	}
}

func clearWorkspace() error {
	session, err := currentTmuxSession()
	if err != nil {
		return err
	}

	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("cannot get current directory: %w", err)
	}
	return clearWorkspaceForSession(session, cwd, true)
}

func resetBranch() error {
	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("cannot get current directory: %w", err)
	}
	return resetGitWorkspace(cwd, true)
}

func setServerSession() error {
	session, err := currentTmuxSession()
	if err != nil {
		return err
	}

	return setServerSessionByName(session)
}

func clearSessionState(session string) error {
	return clearWorkspaceForSession(session, "", false)
}

func closeTmuxSession(session string) error {
	session = strings.TrimSpace(session)
	if session == "" {
		return fmt.Errorf("session required")
	}

	out, err := exec.Command("tmux", "has-session", "-t", session).CombinedOutput()
	if err != nil && !isMissingTmuxSessionOutput(out) {
		return fmt.Errorf("tmux has-session %s: %w: %s", session, err, strings.TrimSpace(string(out)))
	}
	missingSession := err != nil

	homeDir, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot get home directory: %w", err)
	}
	if err := clearSessionStateFiles(homeDir, session); err != nil {
		return err
	}
	if err := removeSessionState(session); err != nil {
		return err
	}
	if missingSession {
		return nil
	}

	out, err = exec.Command("tmux", "kill-session", "-t", session).CombinedOutput()
	if err != nil && !isMissingTmuxSessionOutput(out) {
		return fmt.Errorf("tmux kill-session %s: %w: %s", session, err, strings.TrimSpace(string(out)))
	}
	_ = refreshTmuxClient()
	return nil
}

func isMissingTmuxSessionOutput(out []byte) bool {
	msg := strings.ToLower(string(out))
	return strings.Contains(msg, "can't find session") ||
		strings.Contains(msg, "no such session") ||
		strings.Contains(msg, "no server running")
}

func clearWorkspaceForSession(session, dir string, verbose bool) error {
	if dir != "" {
		if err := resetGitWorkspace(dir, verbose); err != nil {
			return err
		}
	}

	homeDir, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot get home directory: %w", err)
	}
	if err := clearSessionStateFiles(homeDir, session); err != nil {
		return err
	}
	state := loadSessionStateWithLegacy(session)
	state.Label = ""
	state.Server = false
	state.Workspace = ""
	state.AI = map[string]string{}
	if err := saveSessionState(state); err != nil {
		return err
	}
	_ = refreshTmuxClient()
	return nil
}

func clearSessionStateFiles(homeDir, session string) error {
	if err := removeHomeFile(homeDir, ".tmux-label-"+session); err != nil {
		return err
	}
	if err := removeHomeFile(homeDir, ".tmux-server-"+session); err != nil {
		return err
	}
	if err := removeHomeFile(homeDir, ".tmux-workspace-"+session); err != nil {
		return err
	}
	if err := removeHomeFile(homeDir, ".tmux-pr-"+session); err != nil {
		return err
	}
	if err := removeHomeFile(homeDir, ".tmux-claude-"+session); err != nil {
		return err
	}
	if err := removeHomeFilesWithPrefix(homeDir, ".tmux-claude-"+session+"_"); err != nil {
		return err
	}
	if err := removeHomeFile(homeDir, ".tmux-codex-"+session); err != nil {
		return err
	}
	return removeHomeFilesWithPrefix(homeDir, ".tmux-codex-"+session+"_")
}

func setServerSessionByName(session string) error {
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot get home directory: %w", err)
	}

	if err := removeHomeFilesWithPrefix(homeDir, ".tmux-server-"); err != nil {
		return err
	}
	states, err := listSessionStates()
	if err != nil {
		return err
	}
	for i := range states {
		states[i].Server = states[i].Name == session
		if err := saveSessionState(&states[i]); err != nil {
			return err
		}
	}
	state := loadSessionStateWithLegacy(session)
	state.Server = true
	if err := saveSessionState(state); err != nil {
		return err
	}
	if err := writeHomeFile(homeDir, ".tmux-server-"+session, "running\n"); err != nil {
		return err
	}
	return refreshTmuxClient()
}

func currentTmuxSession() (string, error) {
	out, err := exec.Command("tmux", tmuxDisplayArgs("#S")...).Output()
	if err != nil {
		return "", fmt.Errorf("cannot determine current tmux session: %w", err)
	}
	session := strings.TrimSpace(string(out))
	if session == "" {
		return "", fmt.Errorf("cannot determine current tmux session")
	}
	return session, nil
}

func tmuxDisplayArgs(format string) []string {
	if pane := strings.TrimSpace(os.Getenv("TMUX_PANE")); pane != "" {
		return []string{"display-message", "-t", pane, "-p", format}
	}
	return []string{"display-message", "-p", format}
}

func refreshTmuxClient() error {
	cmd := exec.Command("tmux", "refresh-client", "-S")
	out, err := cmd.CombinedOutput()
	if err == nil {
		return nil
	}
	if isNoCurrentClientOutput(out) {
		return nil
	}
	_, _ = os.Stderr.Write(out)
	return err
}

func isNoCurrentClientOutput(out []byte) bool {
	return strings.Contains(string(out), "no current client")
}

func resetGitWorkspace(dir string, verbose bool) error {
	if !insideGitWorktree(dir) {
		if verbose {
			fmt.Println("Not a git repo, skipping git reset")
		}
		return nil
	}

	root, err := gitWorktreeRoot(dir)
	if err != nil {
		return err
	}

	statusOut, err := exec.Command("git", "-C", root, "status", "--porcelain").Output()
	if err != nil {
		return fmt.Errorf("git status: %w", err)
	}
	if len(strings.TrimSpace(string(statusOut))) > 0 {
		return errClearDirty
	}

	base, err := defaultBaseBranch(root)
	if err != nil {
		return err
	}

	dirName := filepath.Base(root)
	if isWorkspaceDir(dirName) {
		if verbose {
			fmt.Printf("Resetting worktree: %s\n", dirName)
		}
		if err := runGitCommand(root, verbose, "checkout", dirName); err != nil {
			return err
		}
		if err := runGitCommand(root, verbose, "fetch", "origin", base); err != nil {
			return err
		}
		return runGitCommand(root, verbose, "reset", "--hard", "origin/"+base)
	}

	if verbose {
		fmt.Printf("Resetting main directory: %s\n", dirName)
	}
	if err := runGitCommand(root, verbose, "checkout", base); err != nil {
		return err
	}
	return runGitCommand(root, verbose, "pull", "origin", base)
}

func insideGitWorktree(dir string) bool {
	cmd := exec.Command("git", "rev-parse", "--is-inside-work-tree")
	cmd.Dir = dir
	return cmd.Run() == nil
}

func gitWorktreeRoot(dir string) (string, error) {
	out, err := exec.Command("git", "-C", dir, "rev-parse", "--show-toplevel").CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git rev-parse --show-toplevel: %w: %s", err, strings.TrimSpace(string(out)))
	}
	root := strings.TrimSpace(string(out))
	if root == "" {
		return "", fmt.Errorf("git rev-parse --show-toplevel returned empty path")
	}
	return root, nil
}

func runGitCommand(dir string, verbose bool, args ...string) error {
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	if verbose {
		cmd.Stdin = os.Stdin
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		return cmd.Run()
	}

	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("git %s: %w: %s", strings.Join(args, " "), err, strings.TrimSpace(string(out)))
	}
	return nil
}

func isWorkspaceDir(name string) bool {
	const prefix = "workspace-"
	if !strings.HasPrefix(name, prefix) {
		return false
	}
	suffix := strings.TrimPrefix(name, prefix)
	if suffix == "" {
		return false
	}
	for _, r := range suffix {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

func writeHomeFile(homeDir, name, contents string) error {
	path := filepath.Join(homeDir, name)
	if err := os.WriteFile(path, []byte(contents), 0644); err != nil {
		return fmt.Errorf("cannot write %s: %w", path, err)
	}
	return nil
}

func removeHomeFile(homeDir, name string) error {
	path := filepath.Join(homeDir, name)
	if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("cannot remove %s: %w", path, err)
	}
	return nil
}

func removeHomeFilesWithPrefix(homeDir, prefix string) error {
	entries, err := os.ReadDir(homeDir)
	if err != nil {
		return fmt.Errorf("cannot read %s: %w", homeDir, err)
	}
	for _, entry := range entries {
		if !strings.HasPrefix(entry.Name(), prefix) {
			continue
		}
		if err := removeHomeFile(homeDir, entry.Name()); err != nil {
			return err
		}
	}
	return nil
}
