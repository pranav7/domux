package main

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
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
		spec, ok := normalizePaneSpec(paneFlag)
		if !ok {
			return commTarget{}, false, fmt.Errorf("invalid --pane %q (want window.pane, e.g. 1.0)", paneFlag)
		}
		t.Pane = spec
		t.Target = s.Name + ":" + spec
		return t, false, nil
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
func resolveLivePane(t commTarget) (commTarget, error) {
	if !tmuxSessionExists(t.Session) {
		return commTarget{}, fmt.Errorf("no worktree or tmux session matches %q (try: domux peek)", t.Name)
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
	return resolveLivePane(t)
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
