package main

import (
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"text/tabwriter"
	"time"
)

// commTarget identifies a peer agent's tmux pane resolved from a worktree name.
type commTarget struct {
	Name    string // the name the caller passed
	Session string // resolved tmux session
	Root    string // worktree root (may be "")
	Label   string // human label, if any
	Pane    string // "1.0"
	Target  string // "workspace-2:1.0"
	Command string // pane_current_command at resolve time (best-effort)
}

// tmuxPane is one row from `tmux list-panes`.
type tmuxPane struct {
	Spec    string // "1.0" (per-session) or "session:1.0" (-a form)
	Command string
	Title   string
	Path    string
}

var (
	paneSpecRe      = regexp.MustCompile(`^\d+[._]\d+$`)
	claudeVersionRe = regexp.MustCompile(`^\d+(\.\d+)+$`)
)

// matchSessionsByName returns states whose name, worktree dir basename, or
// label equal name (case-insensitive).
func matchSessionsByName(name string, states []SessionState) []SessionState {
	want := strings.ToLower(strings.TrimSpace(name))
	if want == "" {
		return nil
	}
	var matches []SessionState
	for _, s := range states {
		if sessionMatchesName(want, s) {
			matches = append(matches, s)
		}
	}
	return matches
}

func sessionMatchesName(want string, s SessionState) bool {
	if strings.ToLower(s.Name) == want {
		return true
	}
	if s.Root != "" && strings.ToLower(filepath.Base(s.Root)) == want {
		return true
	}
	if s.Label != "" && strings.ToLower(strings.TrimSpace(s.Label)) == want {
		return true
	}
	return false
}

// normalizePaneSpec accepts "1_0" or "1.0" and returns "1.0". ok is false for
// anything not window.pane shaped (e.g. "default", "1").
func normalizePaneSpec(s string) (string, bool) {
	s = strings.TrimSpace(s)
	if !paneSpecRe.MatchString(s) {
		return "", false
	}
	return strings.ReplaceAll(s, "_", "."), true
}

// applyExplicitPane sets t.Pane/t.Target from a user-supplied --pane value
// (window.pane). Shared by the state-matched and live-fallback paths so an
// explicit --pane is always honored.
func applyExplicitPane(t commTarget, paneFlag string) (commTarget, error) {
	spec, ok := normalizePaneSpec(paneFlag)
	if !ok {
		return commTarget{}, fmt.Errorf("invalid --pane %q (want window.pane, e.g. 1.0)", paneFlag)
	}
	t.Pane = spec
	t.Target = t.Session + ":" + spec
	return t, nil
}

// claudePaneSpecFromKey turns a SessionState.AI key like "claude:1_0" into the
// pane spec "1.0". ok is false for non-claude or non-pane keys.
func claudePaneSpecFromKey(aiKey string) (string, bool) {
	const prefix = "claude:"
	if !strings.HasPrefix(aiKey, prefix) {
		return "", false
	}
	return normalizePaneSpec(strings.TrimPrefix(aiKey, prefix))
}

// claudePaneSpecsFromState returns the sorted, de-duplicated pane specs of every
// claude pane recorded in the session's AI map.
func claudePaneSpecsFromState(state *SessionState) []string {
	if state == nil {
		return nil
	}
	var specs []string
	seen := map[string]bool{}
	for key := range state.AI {
		if spec, ok := claudePaneSpecFromKey(key); ok && !seen[spec] {
			specs = append(specs, spec)
			seen[spec] = true
		}
	}
	sort.Strings(specs)
	return specs
}

// looksLikeClaudeCommand reports whether a tmux pane_current_command looks like
// a Claude Code agent. Claude reports its version (e.g. "2.1.186"); shells
// report "zsh"/"bash".
func looksLikeClaudeCommand(cmd string) bool {
	cmd = strings.TrimSpace(cmd)
	if cmd == "" {
		return false
	}
	if strings.EqualFold(cmd, "claude") {
		return true
	}
	return claudeVersionRe.MatchString(cmd)
}

// parsePaneLines parses tab-separated "spec\tcommand\ttitle\tpath" lines from a
// `tmux list-panes -F` invocation.
func parsePaneLines(out string) []tmuxPane {
	var panes []tmuxPane
	for _, line := range strings.Split(strings.TrimRight(out, "\n"), "\n") {
		if strings.TrimSpace(line) == "" {
			continue
		}
		f := strings.SplitN(line, "\t", 4)
		p := tmuxPane{Spec: f[0]}
		if len(f) > 1 {
			p.Command = f[1]
		}
		if len(f) > 2 {
			p.Title = f[2]
		}
		if len(f) > 3 {
			p.Path = f[3]
		}
		panes = append(panes, p)
	}
	return panes
}

// splitTarget splits "session:1.0" into ("session", "1.0"). tmux session names
// can't contain ':', so a LastIndex split is safe even for path-ish names.
func splitTarget(target string) (string, string) {
	idx := strings.LastIndex(target, ":")
	if idx < 0 {
		return target, ""
	}
	return target[:idx], target[idx+1:]
}

func ambiguousNameError(name string, matches []SessionState) error {
	var names []string
	for _, m := range matches {
		entry := m.Name
		if m.Label != "" {
			entry = fmt.Sprintf("%s (%s)", m.Name, m.Label)
		}
		names = append(names, entry)
	}
	sort.Strings(names)
	return fmt.Errorf("%q is ambiguous — matches: %s", name, strings.Join(names, ", "))
}

func multiplePanesError(session string, specs []string) error {
	sorted := append([]string(nil), specs...)
	sort.Strings(sorted)
	return fmt.Errorf("session %q has multiple Claude panes (%s); pick one with --pane", session, strings.Join(sorted, ", "))
}

// resolveCommTargetFromStates resolves a name + optional --pane against a
// snapshot of session states, without touching tmux. needsLive is true when the
// pane couldn't be determined from state alone and the caller must scan live
// panes (no claude AI key, or no state matched the name at all).
func resolveCommTargetFromStates(name, paneFlag string, states []SessionState) (commTarget, bool, error) {
	matches := matchSessionsByName(name, states)
	if len(matches) > 1 {
		return commTarget{}, false, ambiguousNameError(name, matches)
	}
	if len(matches) == 0 {
		// No domux state — caller checks for a live tmux session of this name.
		return commTarget{Name: name, Session: name}, true, nil
	}
	s := matches[0]
	t := commTarget{Name: name, Session: s.Name, Root: s.Root, Label: s.Label}

	if paneFlag != "" {
		resolved, err := applyExplicitPane(t, paneFlag)
		if err != nil {
			return commTarget{}, false, err
		}
		return resolved, false, nil
	}

	specs := claudePaneSpecsFromState(&s)
	switch len(specs) {
	case 1:
		t.Pane = specs[0]
		t.Target = s.Name + ":" + specs[0]
		return t, false, nil
	case 0:
		return t, true, nil
	default:
		return commTarget{}, false, multiplePanesError(s.Name, specs)
	}
}

// attributionPrefix builds the header that tells the receiving agent the
// message came from a peer agent, not the human operator.
func attributionPrefix(from string) string {
	from = strings.TrimSpace(from)
	if from == "" {
		return "[domux peer message — from a peer Claude agent, not your operator]"
	}
	return fmt.Sprintf("[domux peer message — from worktree %q, a peer Claude agent (not your operator)]", from)
}

// formatPeerMessage prepends the attribution header to the caller's message.
func formatPeerMessage(from, message string) string {
	return attributionPrefix(from) + "\n\n" + message
}

// listSessionPanes lists the panes of one session (Spec is "window.pane").
func listSessionPanes(session string) ([]tmuxPane, error) {
	out, err := exec.Command("tmux", "list-panes", "-t", session, "-F",
		"#{window_index}.#{pane_index}\t#{pane_current_command}\t#{pane_title}\t#{pane_current_path}").CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("tmux list-panes -t %s: %w: %s", session, err, strings.TrimSpace(string(out)))
	}
	return parsePaneLines(string(out)), nil
}

// listAllPanes lists panes across every session (Spec is "session:window.pane").
func listAllPanes() ([]tmuxPane, error) {
	out, err := exec.Command("tmux", "list-panes", "-a", "-F",
		"#{session_name}:#{window_index}.#{pane_index}\t#{pane_current_command}\t#{pane_title}\t#{pane_current_path}").CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("tmux list-panes -a: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return parsePaneLines(string(out)), nil
}

// paneCurrentCommand returns the foreground command of a tmux pane
// (best-effort; "" when the pane can't be queried).
func paneCurrentCommand(target string) string {
	if target == "" {
		return ""
	}
	out, err := execOutput("tmux", "display-message", "-t", target, "-p", "#{pane_current_command}")
	if err != nil {
		return ""
	}
	return out
}

// resolveLivePane finds the Claude pane in t.Session by scanning live panes,
// used when state alone couldn't pin it down.
func resolveLivePane(t commTarget, paneFlag string) (commTarget, error) {
	if !tmuxSessionExists(t.Session) {
		return commTarget{}, fmt.Errorf("no worktree or tmux session matches %q (try: domux peek)", t.Name)
	}
	if paneFlag != "" {
		resolved, err := applyExplicitPane(t, paneFlag)
		if err != nil {
			return commTarget{}, err
		}
		resolved.Command = paneCurrentCommand(resolved.Target)
		return resolved, nil
	}
	panes, err := listSessionPanes(t.Session)
	if err != nil {
		return commTarget{}, err
	}
	var claude []tmuxPane
	for _, p := range panes {
		if looksLikeClaudeCommand(p.Command) {
			claude = append(claude, p)
		}
	}
	switch {
	case len(claude) == 1:
		t.Pane, t.Command = claude[0].Spec, claude[0].Command
	case len(claude) == 0 && len(panes) == 1:
		t.Pane, t.Command = panes[0].Spec, panes[0].Command
	case len(claude) == 0:
		return commTarget{}, fmt.Errorf("no Claude agent pane found in session %q (use --pane window.pane)", t.Session)
	default:
		specs := make([]string, 0, len(claude))
		for _, p := range claude {
			specs = append(specs, p.Spec)
		}
		return commTarget{}, multiplePanesError(t.Session, specs)
	}
	t.Target = t.Session + ":" + t.Pane
	return t, nil
}

// resolveCommTarget resolves a worktree name (and optional --pane) to a peer
// agent's tmux pane: session state first, live tmux fallback second.
func resolveCommTarget(name, paneFlag string) (commTarget, error) {
	states, err := listSessionStates()
	if err != nil {
		return commTarget{}, err
	}
	t, needsLive, err := resolveCommTargetFromStates(name, paneFlag, states)
	if err != nil {
		return commTarget{}, err
	}
	if !needsLive {
		t.Command = paneCurrentCommand(t.Target)
		return t, nil
	}
	return resolveLivePane(t, paneFlag)
}

// capturePaneTail returns the trimmed last `lines` of a pane (for busy diffing).
func capturePaneTail(target string, lines int) string {
	out, err := execOutput("tmux", "capture-pane", "-t", target, "-p", "-S", fmt.Sprintf("-%d", lines))
	if err != nil {
		return ""
	}
	return out
}

// paneAIState returns the normalized claude AI state for a target pane from
// session state, or "" if unknown/idle.
func paneAIState(session, pane string) string {
	state := loadSessionStateWithLegacy(session)
	key := "claude:" + strings.ReplaceAll(pane, ".", "_")
	return normalizeAIState("claude", state.AI[key])
}

// paneBusyByCapture captures the pane tail twice with a short gap; changed
// output means the pane is actively producing.
func paneBusyByCapture(target string) bool {
	first := capturePaneTail(target, 20)
	time.Sleep(600 * time.Millisecond)
	return first != capturePaneTail(target, 20)
}

// isPaneBusy reports whether the peer agent at target is actively generating.
// domux AI state is the cheap reliable signal when present; otherwise fall back
// to a capture-first diff.
func isPaneBusy(t commTarget) bool {
	switch paneAIState(t.Session, t.Pane) {
	case "CLAUDING", "COMPACTING":
		return true
	case "WAITING":
		return false
	}
	return paneBusyByCapture(t.Target)
}

func tmuxSendLiteral(target, text string) error {
	out, err := exec.Command("tmux", "send-keys", "-t", target, "-l", text).CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux send-keys -l: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func tmuxSendEnter(target string) error {
	out, err := exec.Command("tmux", "send-keys", "-t", target, "Enter").CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux send-keys Enter: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

// isOwnPane reports whether target is the caller's own pane (guards self-sends).
func isOwnPane(target string) bool {
	session, err := currentTmuxSession()
	if err != nil {
		return false
	}
	own := session + ":" + strings.ReplaceAll(currentTmuxPaneKey(), "_", ".")
	return target == own
}

// currentSenderName is the default --from: the caller's domux label, else its
// tmux session name, else "".
func currentSenderName() string {
	session, err := currentTmuxSession()
	if err != nil {
		return ""
	}
	if state := loadSessionStateWithLegacy(session); state.Label != "" {
		return state.Label
	}
	return session
}

// sendCommand: domux send [flags] <worktree> <message…>
// Flags must precede the positionals (Go flag parsing stops at the first
// non-flag arg).
func sendCommand(args []string) error {
	fs := flag.NewFlagSet("send", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	from := fs.String("from", "", "attribution name (defaults to the current session)")
	pane := fs.String("pane", "", "target pane as window.pane, e.g. 1.0")
	noEnter := fs.Bool("no-enter", false, "stage the text in the peer's input box without submitting")
	wait := fs.Bool("wait", false, "wait until the peer is idle before sending")
	waitTimeout := fs.Duration("wait-timeout", 60*time.Second, "max time to wait with --wait")
	if err := fs.Parse(args); err != nil {
		return err
	}
	rest := fs.Args()
	if len(rest) < 2 {
		return fmt.Errorf("usage: domux send [--from NAME] [--pane W.P] [--no-enter] [--wait] <worktree> <message…>")
	}
	name := rest[0]
	message := strings.Join(rest[1:], " ")

	t, err := resolveCommTarget(name, *pane)
	if err != nil {
		return err
	}
	if isOwnPane(t.Target) {
		return fmt.Errorf("refusing to send to your own pane (%s)", t.Target)
	}

	attribution := *from
	if attribution == "" {
		attribution = currentSenderName()
	}

	queued := false
	if *wait {
		deadline := time.Now().Add(*waitTimeout)
		for isPaneBusy(t) {
			if time.Now().After(deadline) {
				return fmt.Errorf("peer %s still busy after %s — not sent (retry, or drop --wait to queue it)", t.Target, *waitTimeout)
			}
			time.Sleep(750 * time.Millisecond)
		}
	} else {
		queued = isPaneBusy(t)
	}

	if err := tmuxSendLiteral(t.Target, formatPeerMessage(attribution, message)); err != nil {
		return err
	}
	if !*noEnter {
		if err := tmuxSendEnter(t.Target); err != nil {
			return err
		}
	}

	switch {
	case *noEnter:
		fmt.Printf("staged in %s (%s) — not submitted (--no-enter)\n", t.Name, t.Target)
	case queued:
		fmt.Printf("sent to %s (%s) — peer was generating, message queued\n", t.Name, t.Target)
	default:
		fmt.Printf("sent to %s (%s)\n", t.Name, t.Target)
	}
	return nil
}

// readCommand: domux read [flags] <worktree>
func readCommand(args []string) error {
	fs := flag.NewFlagSet("read", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	lines := fs.Int("lines", 50, "number of trailing lines to capture")
	pane := fs.String("pane", "", "target pane as window.pane, e.g. 1.0")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("usage: domux read [--lines N] [--pane W.P] <worktree>")
	}
	if *lines < 1 {
		return fmt.Errorf("--lines must be >= 1")
	}
	t, err := resolveCommTarget(fs.Arg(0), *pane)
	if err != nil {
		return err
	}
	out, err := exec.Command("tmux", "capture-pane", "-t", t.Target, "-p", "-S", fmt.Sprintf("-%d", *lines)).CombinedOutput()
	if err != nil {
		return fmt.Errorf("tmux capture-pane -t %s: %w: %s", t.Target, err, strings.TrimSpace(string(out)))
	}
	os.Stdout.Write(out)
	return nil
}

// peekCommand: domux peek — list running Claude agents across worktrees.
func peekCommand(args []string) error {
	if len(args) != 0 {
		return fmt.Errorf("peek does not accept arguments")
	}
	panes, err := listAllPanes()
	if err != nil {
		return err
	}
	states, err := listSessionStates()
	if err != nil {
		return err
	}
	rows := peekRows(panes, states)
	if len(rows) == 0 {
		fmt.Println("no running Claude agents found")
		return nil
	}
	w := tabwriter.NewWriter(os.Stdout, 0, 2, 2, ' ', 0)
	fmt.Fprintln(w, "NAME\tSTATE\tTASK\tTARGET")
	for _, r := range rows {
		name := r.Name
		if r.Label != "" {
			name = fmt.Sprintf("%s (%s)", r.Name, r.Label)
		}
		fmt.Fprintf(w, "%s\t%s\t%s\t%s\n", name, r.State, truncateTask(r.Task), r.Target)
	}
	return w.Flush()
}

// peekRow is one line of `domux peek` output.
type peekRow struct {
	Name   string // addressable session name
	Label  string // human label, if any
	Target string // session:pane
	State  string // working / waiting / compacting / idle / unknown
	Task   string // pane title (the agent's current task)
}

// peekRows turns live panes + session states into sorted peek rows, keeping only
// panes that look like Claude agents.
func peekRows(panes []tmuxPane, states []SessionState) []peekRow {
	byName := map[string]SessionState{}
	for _, s := range states {
		byName[s.Name] = s
	}
	var rows []peekRow
	for _, p := range panes {
		if !looksLikeClaudeCommand(p.Command) {
			continue
		}
		session, paneSpec := splitTarget(p.Spec)
		row := peekRow{Name: session, Target: p.Spec, Task: p.Title, State: "unknown"}
		if s, ok := byName[session]; ok {
			row.Label = s.Label
			row.State = peekStateLabel(&s, paneSpec)
		}
		rows = append(rows, row)
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].Target < rows[j].Target })
	return rows
}

// peekStateLabel maps a pane's claude AI state to a peek-friendly word.
func peekStateLabel(state *SessionState, paneSpec string) string {
	key := "claude:" + strings.ReplaceAll(paneSpec, ".", "_")
	switch normalizeAIState("claude", state.AI[key]) {
	case "CLAUDING":
		return "working"
	case "WAITING":
		return "waiting"
	case "COMPACTING":
		return "compacting"
	default:
		return "idle"
	}
}

func truncateTask(s string) string {
	s = strings.TrimSpace(s)
	r := []rune(s)
	if len(r) > 50 {
		return string(r[:47]) + "..."
	}
	return s
}
