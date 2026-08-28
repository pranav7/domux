package main

import (
	"fmt"
	"html"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

const artboardsDecayDays = 14

var artboardsStates = []string{"agreed", "pending", "open"}

const artboardsDefaultState = "pending"

// artboardsStateWidth pads state tokens so they line up down the page —
// ports STATE_WIDTH = max(len(s) for s in STATES) from .index-gen.py.
var artboardsStateWidth = func() int {
	w := 0
	for _, s := range artboardsStates {
		if len(s) > w {
			w = len(s)
		}
	}
	return w
}()

type artboardRow struct {
	Path  string
	Title string
	State string
	Note  string
	MTime time.Time
}

type artboardGroup struct {
	Name      string // folder name (the session+title key)
	Title     string
	Workspace string
	Session   string
	Repo      string
	Branch    string
	Rows      []artboardRow
	Newest    time.Time
	Archived  bool
	Decayed   bool
	Missing   int
}

// artboardStem: "Main.dc.html" -> "Main".
func artboardStem(name string) string {
	if strings.HasSuffix(name, ".dc.html") {
		return strings.TrimSuffix(name, ".dc.html")
	}
	return strings.TrimSuffix(name, filepath.Ext(name))
}

func isArtboardState(s string) bool {
	for _, v := range artboardsStates {
		if s == v {
			return true
		}
	}
	return false
}

// artboardFilesOnDisk lists every artboard document in a group folder,
// alphabetical — ports board_files.
func artboardFilesOnDisk(groupDir string) []string {
	entries, err := os.ReadDir(groupDir)
	if err != nil {
		return nil
	}
	var names []string
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		name := e.Name()
		if name == "index.html" || !strings.HasSuffix(strings.ToLower(name), ".html") {
			continue
		}
		names = append(names, name)
	}
	sort.Slice(names, func(i, j int) bool {
		return strings.ToLower(names[i]) < strings.ToLower(names[j])
	})
	return names
}

// artboardBoardRows ports board_rows: rows in authored order (from
// meta.Artboards), then any unlisted on-disk file, alphabetical.
func artboardBoardRows(groupDir string, meta ArtboardsMeta) ([]artboardRow, int) {
	onDisk := artboardFilesOnDisk(groupDir)
	onDiskSet := make(map[string]bool, len(onDisk))
	for _, n := range onDisk {
		onDiskSet[n] = true
	}

	claimed := map[string]bool{}
	missing := 0
	var rows []artboardRow
	for _, entry := range meta.Artboards {
		if entry.File == "" || !onDiskSet[entry.File] || claimed[entry.File] {
			missing++
			continue
		}
		claimed[entry.File] = true
		state := entry.State
		if !isArtboardState(state) {
			state = artboardsDefaultState
		}
		title := entry.Title
		if title == "" {
			title = artboardStem(entry.File)
		}
		path := filepath.Join(groupDir, entry.File)
		info, _ := os.Stat(path)
		rows = append(rows, artboardRow{
			Path:  path,
			Title: title,
			State: state,
			Note:  entry.Note,
			MTime: mtimeOf(info),
		})
	}
	for _, name := range onDisk {
		if claimed[name] {
			continue
		}
		path := filepath.Join(groupDir, name)
		info, _ := os.Stat(path)
		rows = append(rows, artboardRow{
			Path:  path,
			Title: artboardStem(name),
			State: artboardsDefaultState,
			MTime: mtimeOf(info),
		})
	}
	return rows, missing
}

func mtimeOf(info os.FileInfo) time.Time {
	if info == nil {
		return time.Time{}
	}
	return info.ModTime()
}

func firstNonEmpty(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// readArtboardGroup ports read_group: one group, or nil when the folder
// holds no artboards at all.
func readArtboardGroup(groupDir string, now time.Time) *artboardGroup {
	meta := readArtboardsMeta(groupDir)
	rows, missing := artboardBoardRows(groupDir, meta)
	if len(rows) == 0 {
		return nil
	}
	var newest time.Time
	for _, r := range rows {
		if r.MTime.After(newest) {
			newest = r.MTime
		}
	}
	stale := now.Sub(newest) > artboardsDecayDays*24*time.Hour
	name := filepath.Base(groupDir)
	return &artboardGroup{
		Name:      name,
		Title:     firstNonEmpty(meta.Title, name),
		Workspace: meta.Workspace,
		Session:   meta.Session, // empty is filled with Name at render time — never a KeyError like the Python original
		Repo:      meta.Repo,
		Branch:    meta.Branch,
		Rows:      rows,
		Newest:    newest,
		Archived:  !meta.Active || stale,
		Decayed:   stale,
		Missing:   missing,
	}
}

// safeReadArtboardGroup wraps readArtboardGroup in its own recover — the
// honest Go equivalent of the Python generator's blanket per-group
// except-Exception, so one bad group can never take the whole page down.
func safeReadArtboardGroup(groupDir string, now time.Time) (group *artboardGroup) {
	defer func() {
		if r := recover(); r != nil {
			fmt.Fprintf(os.Stderr, "artboards: skipped %s: %v\n", filepath.Base(groupDir), r)
			group = nil
		}
	}()
	return readArtboardGroup(groupDir, now)
}

// readArtboardGroups ports read_groups, including its final sort: cluster by
// workspace (empty sinks to the end, like "~" sorting after real names), then
// newest first inside each.
func readArtboardGroups(root string) []artboardGroup {
	entries, err := os.ReadDir(root)
	if err != nil {
		if !os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "artboards: cannot read %s: %v\n", root, err)
		}
		return nil
	}
	names := make([]string, 0, len(entries))
	for _, e := range entries {
		names = append(names, e.Name())
	}
	sort.Slice(names, func(i, j int) bool {
		return strings.ToLower(names[i]) < strings.ToLower(names[j])
	})

	now := time.Now()
	var groups []artboardGroup
	for _, name := range names {
		if strings.HasPrefix(name, ".") {
			continue
		}
		groupDir := filepath.Join(root, name)
		info, err := os.Stat(groupDir) // follows symlinks — how a symlinked group is legitimately included
		if err != nil || !info.IsDir() {
			continue
		}
		if g := safeReadArtboardGroup(groupDir, now); g != nil {
			groups = append(groups, *g)
		}
	}
	sort.SliceStable(groups, func(i, j int) bool {
		wi, wj := groups[i].Workspace, groups[j].Workspace
		if wi == "" {
			wi = "~"
		}
		if wj == "" {
			wj = "~"
		}
		if wi != wj {
			return wi < wj
		}
		return groups[i].Newest.After(groups[j].Newest)
	})
	return groups
}

// artboardsCSS is copied verbatim from the settled .index-gen.py design —
// pill badges for the group's repo/workspace tags, plain colored text (no
// pill) for the per-row state tokens.
const artboardsCSS = `
:root{
  --ground:#fbfaf9; --surface:#ffffff; --ink:#1c1917; --dim:#78716c;
  --faint:#a8a29e; --rule:#e7e5e4; --accent:#6d4aff;
  --agreed:#15803d; --pending:#b45309; --open:#a16207;
  --tag-bg:#efeeec; --accent-bg:#ece7fb;
}
@media (prefers-color-scheme: dark){
  :root{
    --ground:#0d0d0f; --surface:#15151a; --ink:#e7e5e4; --dim:#8b8781;
    --faint:#52525b; --rule:#27272a; --accent:#a78bfa;
    --agreed:#4ade80; --pending:#fbbf24; --open:#facc15;
    --tag-bg:#202024; --accent-bg:#2a2340;
  }
}
*{box-sizing:border-box}
body{
  margin:0; background:var(--ground); color:var(--ink);
  font:500 13px/1.55 "IBM Plex Mono", ui-monospace, SFMono-Regular, 'SF Mono', Menlo, monospace;
  font-variant-numeric:tabular-nums;
}
.page{display:flex; flex-direction:column; gap:40px; max-width:78ch; padding:72px 32px 96px}
.head{display:flex; flex-direction:column; gap:6px}
h1{margin:0; font-size:15px; font-weight:600; letter-spacing:.01em}
.meta{margin:0; font-size:11.5px; font-weight:500; color:var(--dim)}
.bit{white-space:nowrap}
.sep{color:var(--faint)}
.groups{display:flex; flex-direction:column; gap:34px}
.group{display:flex; flex-direction:column; gap:12px; border-top:1px solid var(--rule); padding-top:16px}
.group-head{display:flex; flex-direction:column; gap:4px}
.group-title{display:flex; align-items:baseline; flex-wrap:wrap; gap:8px}
.name{font-size:13px; font-weight:600}
.pill{display:inline-flex; align-items:center; padding:1px 8px; border-radius:999px}
.tag{font-size:11.5px; font-weight:500; color:var(--dim); background:var(--tag-bg)}
.tag.ws{color:var(--accent); background:var(--accent-bg)}
.rows{display:flex; flex-direction:column; gap:1px}
.row{
  display:grid; grid-template-columns:auto 1fr auto; column-gap:12px; row-gap:1px;
  padding:5px 8px; margin:0 -8px; border-radius:3px;
  color:var(--ink); text-decoration:none;
}
.row:hover{background:var(--surface)}
.row:focus-visible{outline:1.5px solid var(--accent); outline-offset:1px}
@media (prefers-reduced-motion: no-preference){
  .row{transition:background-color 120ms ease}
}
.tok{white-space:pre; font-weight:500}
.tok.st-agreed{color:var(--agreed)}
.tok.st-pending{color:var(--pending)}
.tok.st-open{color:var(--open)}
.title{font-weight:500}
.when{font-size:11.5px; font-weight:500; color:var(--dim); white-space:nowrap; align-self:baseline}
.note{grid-column:2 / 4; font-size:11.5px; font-weight:500; color:var(--dim)}
details{border-top:1px solid var(--rule); padding-top:16px}
summary{font-size:11.5px; font-weight:500; color:var(--dim); cursor:pointer}
summary:focus-visible{outline:1.5px solid var(--accent); outline-offset:2px}
details > .groups{padding-top:26px}
`

func artboardsWhen(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return strings.ToLower(t.Format("02 Jan 15:04"))
}

func artboardsPlural(n int, word string) string {
	if n == 1 {
		return fmt.Sprintf("%d %s", n, word)
	}
	return fmt.Sprintf("%d %ss", n, word)
}

func artboardsHref(groupName, fileName string) string {
	return html.EscapeString("/" + url.PathEscape(groupName) + "/" + url.PathEscape(fileName))
}

// artboardsBitSpan ports bit_span: one meta-line field, each carrying its own
// leading separator so a line break lands between fields, never inside one.
func artboardsBitSpan(text string, first bool) string {
	sep := ""
	if !first {
		sep = `<span class="sep">·</span> `
	}
	return fmt.Sprintf(`<span class="bit">%s%s</span>`, sep, html.EscapeString(text))
}

func artboardsPadRight(s string, width int) string {
	if len(s) >= width {
		return s
	}
	return s + strings.Repeat(" ", width-len(s))
}

func artboardsRenderRow(group artboardGroup, row artboardRow) string {
	token := fmt.Sprintf(`<span class="tok st-%s">%s</span>`, row.State, artboardsPadRight(row.State, artboardsStateWidth))
	note := ""
	if row.Note != "" {
		note = fmt.Sprintf(`<span class="note">%s</span>`, html.EscapeString(row.Note))
	}
	href := artboardsHref(group.Name, filepath.Base(row.Path))
	return fmt.Sprintf(`<a class="row" href="%s">%s<span class="title">%s</span><span class="when">%s</span>%s</a>`,
		href, token, html.EscapeString(row.Title), artboardsWhen(row.MTime), note)
}

func artboardsRenderGroup(group artboardGroup) string {
	// Identity line is the session (falling back to the folder name — unlike
	// the Python original, which referenced a "dirname" key read_group never
	// set and would KeyError on an empty session), then repo/workspace pills.
	session := firstNonEmpty(group.Session, group.Name)
	parts := []string{fmt.Sprintf(`<span class="name">%s</span>`, html.EscapeString(session))}
	if group.Repo != "" {
		parts = append(parts, fmt.Sprintf(`<span class="tag pill">%s</span>`, html.EscapeString(group.Repo)))
	}
	if group.Workspace != "" {
		parts = append(parts, fmt.Sprintf(`<span class="tag ws pill">%s</span>`, html.EscapeString(group.Workspace)))
	}

	var bits []string
	if group.Title != "" {
		bits = append(bits, group.Title)
	}
	if group.Branch != "" {
		bits = append(bits, group.Branch)
	}
	bits = append(bits, "last change "+artboardsWhen(group.Newest))

	bitSpans := make([]string, len(bits))
	for i, b := range bits {
		bitSpans[i] = artboardsBitSpan(b, i == 0)
	}

	var rows strings.Builder
	for _, r := range group.Rows {
		rows.WriteString(artboardsRenderRow(group, r))
	}

	return fmt.Sprintf(`<section class="group"><div class="group-head">`+
		`<div class="group-title">%s</div>`+
		`<p class="meta">%s</p></div>`+
		`<div class="rows">%s</div></section>`,
		strings.Join(parts, ""), strings.Join(bitSpans, " "), rows.String())
}

func artboardsRenderPage(groups []artboardGroup) string {
	var active, archived []artboardGroup
	for _, g := range groups {
		if g.Archived {
			archived = append(archived, g)
		} else {
			active = append(active, g)
		}
	}

	var lead, body string
	if len(active) > 0 {
		boards := 0
		for _, g := range active {
			boards += len(g.Rows)
		}
		lead = fmt.Sprintf(`<p class="meta">%s · %s</p>`, artboardsPlural(len(active), "active group"), artboardsPlural(boards, "artboard"))
		var b strings.Builder
		for _, g := range active {
			b.WriteString(artboardsRenderGroup(g))
		}
		body = `<div class="groups">` + b.String() + `</div>`
	} else {
		lead = `<p class="meta">no active groups</p>`
	}

	if len(archived) > 0 {
		var b strings.Builder
		for _, g := range archived {
			b.WriteString(artboardsRenderGroup(g))
		}
		body += fmt.Sprintf(`<details><summary>%s</summary><div class="groups">%s</div></details>`,
			artboardsPlural(len(archived), "archived group"), b.String())
	}

	return "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">" +
		"<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">" +
		"<title>artboards</title>" +
		"<link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">" +
		"<link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>" +
		"<link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?" +
		"family=IBM+Plex+Mono:wght@400;500;600&display=swap\">" +
		"<style>" + artboardsCSS + "</style></head><body><main class=\"page\">" +
		"<div class=\"head\"><h1>artboards</h1>" + lead + "</div>" +
		body + "</main></body></html>\n"
}

// regenerateArtboardsIndex writes index.html atomically (tmp+rename) — a
// correctness improvement over the Python original's plain write_text, so no
// request can ever observe a half-written file.
func regenerateArtboardsIndex(root string) (string, error) {
	groups := readArtboardGroups(root)
	page := artboardsRenderPage(groups)

	outPath := filepath.Join(root, "index.html")
	tmp := outPath + ".tmp"
	if err := os.WriteFile(tmp, []byte(page), 0644); err != nil {
		return "", fmt.Errorf("cannot write %s: %w", tmp, err)
	}
	if err := os.Rename(tmp, outPath); err != nil {
		_ = os.Remove(tmp)
		return "", fmt.Errorf("cannot rename %s: %w", tmp, err)
	}

	active, boards, decayed, missing := 0, 0, 0, 0
	for _, g := range groups {
		if !g.Archived {
			active++
			boards += len(g.Rows)
		}
		if g.Decayed {
			decayed++
		}
		missing += g.Missing
	}
	summary := fmt.Sprintf("artboards index: %s, %s, %d archived (%d by decay), %d metadata entries skipped",
		artboardsPlural(active, "active group"), artboardsPlural(boards, "artboard"), len(groups)-active, decayed, missing)
	return summary, nil
}
