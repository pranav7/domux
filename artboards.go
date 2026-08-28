package main

import (
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

// ArtboardsMetaEntry is one artboard row inside a group's meta.json.
type ArtboardsMetaEntry struct {
	File  string `json:"file"`
	Title string `json:"title,omitempty"`
	State string `json:"state,omitempty"`
	Note  string `json:"note,omitempty"`
}

// ArtboardsMeta is a group's meta.json — see .index-gen.py's original module
// docstring for the schema this mirrors. Every field is optional; a group
// with no meta.json at all reads as a zero value with Active true.
type ArtboardsMeta struct {
	Session   string               `json:"session,omitempty"`
	Workspace string               `json:"workspace,omitempty"`
	Repo      string               `json:"repo,omitempty"`
	Branch    string               `json:"branch,omitempty"`
	Title     string               `json:"title,omitempty"`
	Active    bool                 `json:"active"`
	Artboards []ArtboardsMetaEntry `json:"artboards,omitempty"`
}

func artboardsRoot() (string, error) {
	return domuxDataDir("artboards")
}

func artboardsMetaPath(groupDir string) string {
	return filepath.Join(groupDir, "meta.json")
}

// readArtboardsMeta decodes meta.json the way the original Python generator
// did: a missing file is {}, and a malformed file — bad JSON, JSON that isn't
// an object, or any single field with the wrong JSON type — degrades just
// that file/field rather than aborting. A plain json.Unmarshal into
// ArtboardsMeta fails the whole decode on one wrong-typed field, so fields
// are pulled one at a time from a raw map instead, matching the
// isinstance-per-field leniency the Python read_meta/board_rows relied on.
func readArtboardsMeta(groupDir string) ArtboardsMeta {
	meta := ArtboardsMeta{Active: true}
	data, err := os.ReadFile(artboardsMetaPath(groupDir))
	if err != nil {
		return meta
	}
	var raw map[string]any
	if err := json.Unmarshal(data, &raw); err != nil {
		fmt.Fprintf(os.Stderr, "artboards: bad meta.json in %s: %v\n", filepath.Base(groupDir), err)
		return meta
	}
	meta.Session = stringField(raw, "session")
	meta.Workspace = stringField(raw, "workspace")
	meta.Repo = stringField(raw, "repo")
	meta.Branch = stringField(raw, "branch")
	meta.Title = stringField(raw, "title")
	if active, ok := raw["active"].(bool); ok {
		meta.Active = active
	}
	if entries, ok := raw["artboards"].([]any); ok {
		for _, e := range entries {
			entryMap, ok := e.(map[string]any)
			if !ok {
				continue
			}
			meta.Artboards = append(meta.Artboards, ArtboardsMetaEntry{
				File:  stringField(entryMap, "file"),
				Title: stringField(entryMap, "title"),
				State: stringField(entryMap, "state"),
				Note:  stringField(entryMap, "note"),
			})
		}
	}
	return meta
}

func stringField(m map[string]any, key string) string {
	s, _ := m[key].(string)
	return s
}

func writeArtboardsMeta(groupDir string, meta ArtboardsMeta) error {
	data, err := json.MarshalIndent(meta, "", "  ")
	if err != nil {
		return fmt.Errorf("cannot encode meta.json: %w", err)
	}
	data = append(data, '\n')
	path := artboardsMetaPath(groupDir)
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

// artboardsGroupKey is the deterministic, filesystem- and URL-safe folder
// name for a (session, title) pair. Pure function of its inputs — the same
// session+title always produces the same key, so re-running `artboards init`
// with the same title resumes the existing group instead of duplicating it.
// The hash suffix means two titles that happen to slugify identically still
// get different keys, without a filesystem collision check at call time.
func artboardsGroupKey(session, title string) string {
	sum := sha1.Sum([]byte(session + "\x00" + title))
	return fmt.Sprintf("%s--%s--%s", artboardsSlug(session), artboardsSlug(title), hex.EncodeToString(sum[:])[:8])
}

// artboardsSlug lowercases and keeps only [a-z0-9], collapsing every other
// run of characters to a single "-". Unlike sanitizeSessionName (which only
// escapes "/" and ":"), this can never emit ".." or "/" — both the folder
// name and a URL path segment need that stronger guarantee.
func artboardsSlug(s string) string {
	var b strings.Builder
	dash := true // seed true so a leading non-alnum run never emits a leading "-"
	for _, r := range strings.ToLower(s) {
		if r >= 'a' && r <= 'z' || r >= '0' && r <= '9' {
			b.WriteRune(r)
			dash = false
			continue
		}
		if !dash {
			b.WriteByte('-')
			dash = true
		}
	}
	slug := strings.Trim(b.String(), "-")
	if len(slug) > 40 {
		slug = strings.TrimRight(slug[:40], "-")
	}
	if slug == "" {
		return "untitled"
	}
	return slug
}

func artboardsCommand(args []string) error {
	if len(args) == 0 {
		return fmt.Errorf(`usage: domux artboards init "<title>"|list|archive|activate|status|restart|open|serve|migrate`)
	}
	switch args[0] {
	case "init":
		return artboardsInitCommand(args[1:])
	case "list":
		return artboardsListCommand(args[1:])
	case "archive":
		return artboardsSetActiveCommand(args[1:], false)
	case "activate":
		return artboardsSetActiveCommand(args[1:], true)
	case "status":
		return artboardsStatusCommand(args[1:])
	case "restart":
		return artboardsRestartCommand(args[1:])
	case "open":
		return artboardsOpenCommand(args[1:])
	case "serve":
		return artboardsServeCommand(args[1:])
	case "migrate":
		return artboardsMigrateCommand(args[1:])
	default:
		return fmt.Errorf("unknown artboards subcommand %q", args[0])
	}
}

// artboardsInitCommand registers or resumes a group for the current domux
// session. Attribution (session/workspace/repo/branch) is always derived
// here, never taken from a flag — an agent is not trusted to name its own
// workspace. Resuming an existing group only patches attribution fields;
// Active and Artboards are left untouched so a second init never wipes
// recorded review state.
func artboardsInitCommand(args []string) error {
	fs := flag.NewFlagSet("artboards init", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf(`usage: domux artboards init "<title>"`)
	}
	title := fs.Arg(0)

	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("cannot get current directory: %w", err)
	}
	ctx, err := resolveDomuxContext(cwd)
	if err != nil {
		return err
	}
	if ctx.Session == "" {
		return fmt.Errorf("cannot determine domux session - run this from inside a domux-managed tmux pane")
	}

	repo := ""
	if ctx.Root != "" {
		repo = filepath.Base(ctx.Root)
	}
	branch := gitBranch(ctx.Root)
	workspace := ""
	if ctx.State != nil {
		workspace = ctx.State.Workspace
	}

	root, err := artboardsRoot()
	if err != nil {
		return err
	}
	groupDir := filepath.Join(root, artboardsGroupKey(ctx.Session, title))

	meta := readArtboardsMeta(groupDir)
	meta.Session = ctx.Session
	meta.Workspace = workspace
	meta.Repo = repo
	meta.Branch = branch
	meta.Title = title

	if err := os.MkdirAll(groupDir, 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", groupDir, err)
	}
	if err := writeArtboardsMeta(groupDir, meta); err != nil {
		return err
	}

	fmt.Println(groupDir)
	fmt.Fprintf(os.Stderr, "session=%s workspace=%s repo=%s branch=%s title=%q\n", ctx.Session, workspace, repo, branch, title)
	return nil
}

type artboardsListEntry struct {
	Key       string `json:"key"`
	Session   string `json:"session"`
	Workspace string `json:"workspace"`
	Repo      string `json:"repo"`
	Branch    string `json:"branch"`
	Title     string `json:"title"`
	Active    bool   `json:"active"`
	Boards    int    `json:"boards"`
}

func artboardsListCommand(args []string) error {
	fs := flag.NewFlagSet("artboards list", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	all := fs.Bool("all", false, "include archived groups")
	archivedOnly := fs.Bool("archived", false, "only archived groups")
	asJSON := fs.Bool("json", false, "print as JSON")
	if err := fs.Parse(args); err != nil {
		return err
	}
	root, err := artboardsRoot()
	if err != nil {
		return err
	}

	var entries []artboardsListEntry
	for _, g := range readArtboardGroups(root) {
		if *archivedOnly && !g.Archived {
			continue
		}
		if !*all && !*archivedOnly && g.Archived {
			continue
		}
		entries = append(entries, artboardsListEntry{
			Key:       g.Name,
			Session:   g.Session,
			Workspace: g.Workspace,
			Repo:      g.Repo,
			Branch:    g.Branch,
			Title:     g.Title,
			Active:    !g.Archived,
			Boards:    len(g.Rows),
		})
	}

	if *asJSON {
		data, err := json.MarshalIndent(entries, "", "  ")
		if err != nil {
			return err
		}
		fmt.Println(string(data))
		return nil
	}
	if len(entries) == 0 {
		fmt.Println("no artboards groups")
		return nil
	}
	for _, e := range entries {
		state := "active"
		if !e.Active {
			state = "archived"
		}
		fmt.Printf("%-8s %-2d boards  %-40s %s\n", state, e.Boards, e.Key, e.Title)
	}
	return nil
}

func artboardsSetActiveCommand(args []string, active bool) error {
	verb := "archive"
	if active {
		verb = "activate"
	}
	if len(args) != 1 {
		return fmt.Errorf("usage: domux artboards %s <key>", verb)
	}
	root, err := artboardsRoot()
	if err != nil {
		return err
	}
	groupDir := filepath.Join(root, args[0])
	if !dirExists(groupDir) {
		return fmt.Errorf("no artboards group %q", args[0])
	}
	meta := readArtboardsMeta(groupDir)
	meta.Active = active
	if err := writeArtboardsMeta(groupDir, meta); err != nil {
		return err
	}
	state := "archived"
	if active {
		state = "activated"
	}
	fmt.Printf("%s %s\n", state, args[0])
	return nil
}

func artboardsOpenCommand(args []string) error {
	if len(args) != 0 {
		return fmt.Errorf("open does not accept arguments")
	}
	url := fmt.Sprintf("http://127.0.0.1:%d/", artboardsPort)
	if runtime.GOOS == "darwin" {
		if err := exec.Command("open", url).Start(); err == nil {
			return nil
		}
	}
	fmt.Println(url)
	return nil
}

// artboardsMigrateCommand moves legacy session-only groups under
// ~/.domux/artboards to their session+title key under domux's real data
// dir. It renames the group entry itself (os.Rename on a symlink renames the
// link, never touching its target) — a symlinked group's real files are
// never read, copied, or rewritten by this command.
func artboardsMigrateCommand(args []string) error {
	fs := flag.NewFlagSet("artboards migrate", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	apply := fs.Bool("apply", false, "perform the moves (default: preview only)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	homeDir, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot get home directory: %w", err)
	}
	legacyRoot := filepath.Join(homeDir, ".domux", "artboards")
	newRoot, err := artboardsRoot()
	if err != nil {
		return err
	}

	entries, err := os.ReadDir(legacyRoot)
	if os.IsNotExist(err) {
		fmt.Println("no legacy artboards root found - nothing to migrate")
		return nil
	}
	if err != nil {
		return fmt.Errorf("cannot read %s: %w", legacyRoot, err)
	}

	moved := 0
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), ".") {
			continue // .server.py, .index-gen.py, .server.log
		}
		if e.Type().IsRegular() {
			continue // index.html - a generated file, not a group. A symlinked
			// group's DirEntry.IsDir() is false (Lstat-based, not resolved), so
			// filtering on IsDir() instead would skip the very group being
			// migrated - IsRegular() is the check that actually excludes only
			// plain files while keeping directories and symlinks.
		}
		oldPath := filepath.Join(legacyRoot, e.Name())
		meta := readArtboardsMeta(oldPath) // reads straight through the symlink
		session := meta.Session
		if session == "" {
			session = e.Name()
		}
		title := meta.Title
		if title == "" {
			title = e.Name()
		}
		newPath := filepath.Join(newRoot, artboardsGroupKey(session, title))
		if _, err := os.Lstat(newPath); err == nil {
			return fmt.Errorf("refusing to migrate %s: %s already exists", e.Name(), newPath)
		}
		fmt.Printf("%s -> %s\n", oldPath, newPath)
		if *apply {
			if err := os.MkdirAll(newRoot, 0755); err != nil {
				return err
			}
			if err := os.Rename(oldPath, newPath); err != nil {
				return fmt.Errorf("rename %s: %w", e.Name(), err)
			}
		}
		moved++
	}

	if moved == 0 {
		fmt.Println("no legacy groups found - nothing to migrate")
		return nil
	}
	if !*apply {
		fmt.Println("preview only - pass --apply to perform the moves")
	}
	return nil
}
