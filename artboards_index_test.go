package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func writeArtboardFile(t *testing.T, dir, name string) {
	t.Helper()
	if err := os.WriteFile(filepath.Join(dir, name), []byte("<html></html>"), 0644); err != nil {
		t.Fatalf("write %s: %v", name, err)
	}
}

func TestArtboardBoardRowsArrayOrderWinsOverAlphabetical(t *testing.T) {
	dir := t.TempDir()
	// Alphabetically "BandOptions" sorts before "Main" — the array order must win.
	writeArtboardFile(t, dir, "Main.dc.html")
	writeArtboardFile(t, dir, "BandOptions.dc.html")

	meta := ArtboardsMeta{Artboards: []ArtboardsMetaEntry{
		{File: "Main.dc.html", State: "agreed"},
		{File: "BandOptions.dc.html", State: "agreed"},
	}}
	rows, missing := artboardBoardRows(dir, meta)
	if missing != 0 {
		t.Fatalf("expected no missing entries, got %d", missing)
	}
	if len(rows) != 2 || filepath.Base(rows[0].Path) != "Main.dc.html" || filepath.Base(rows[1].Path) != "BandOptions.dc.html" {
		t.Fatalf("expected array order preserved, got %+v", rows)
	}
}

func TestArtboardBoardRowsMissingAndUnlistedFiles(t *testing.T) {
	dir := t.TempDir()
	writeArtboardFile(t, dir, "Main.dc.html")
	writeArtboardFile(t, dir, "Extra.dc.html") // not in the array at all

	meta := ArtboardsMeta{Artboards: []ArtboardsMetaEntry{
		{File: "Main.dc.html", State: "agreed"},
		{File: "GoneFromDisk.dc.html", State: "agreed"}, // named but not on disk
	}}
	rows, missing := artboardBoardRows(dir, meta)
	if missing != 1 {
		t.Fatalf("expected 1 missing entry, got %d", missing)
	}
	if len(rows) != 2 {
		t.Fatalf("expected Main + the unlisted Extra, got %+v", rows)
	}
	if filepath.Base(rows[0].Path) != "Main.dc.html" || rows[0].State != "agreed" {
		t.Fatalf("expected Main first with its authored state, got %+v", rows[0])
	}
	if filepath.Base(rows[1].Path) != "Extra.dc.html" || rows[1].State != artboardsDefaultState || rows[1].Note != "" {
		t.Fatalf("expected the unlisted file trailing as pending/no-note, got %+v", rows[1])
	}
}

func TestArtboardBoardRowsUnknownStateReadsAsPending(t *testing.T) {
	dir := t.TempDir()
	writeArtboardFile(t, dir, "Main.dc.html")
	meta := ArtboardsMeta{Artboards: []ArtboardsMetaEntry{{File: "Main.dc.html", State: "bogus"}}}
	rows, _ := artboardBoardRows(dir, meta)
	if len(rows) != 1 || rows[0].State != artboardsDefaultState {
		t.Fatalf("expected an unknown state to normalize to %q, got %+v", artboardsDefaultState, rows)
	}
}

func setMtime(t *testing.T, path string, when time.Time) {
	t.Helper()
	if err := os.Chtimes(path, when, when); err != nil {
		t.Fatalf("Chtimes %s: %v", path, err)
	}
}

func TestReadArtboardGroupDecayBoundary(t *testing.T) {
	now := time.Now()

	freshDir := t.TempDir()
	writeArtboardFile(t, freshDir, "Main.dc.html")
	setMtime(t, filepath.Join(freshDir, "Main.dc.html"), now.Add(-artboardsDecayDays*24*time.Hour+time.Minute))
	writeFile(t, filepath.Join(freshDir, "meta.json"), `{"active":true}`)
	fresh := readArtboardGroup(freshDir, now)
	if fresh == nil || fresh.Archived {
		t.Fatalf("expected a group just inside the decay window to stay active, got %+v", fresh)
	}

	staleDir := t.TempDir()
	writeArtboardFile(t, staleDir, "Main.dc.html")
	setMtime(t, filepath.Join(staleDir, "Main.dc.html"), now.Add(-artboardsDecayDays*24*time.Hour-time.Minute))
	writeFile(t, filepath.Join(staleDir, "meta.json"), `{"active":true}`)
	stale := readArtboardGroup(staleDir, now)
	if stale == nil || !stale.Archived || !stale.Decayed {
		t.Fatalf("expected a group just past the decay window to archive by decay, got %+v", stale)
	}
}

func TestReadArtboardGroupEmptySessionFallsBackToFolderNameNoPanic(t *testing.T) {
	// Regression: the Python original read a "dirname" key read_group never
	// set and would KeyError on an empty session, freezing the whole index.
	// The Go port must fall back to the folder name instead of panicking.
	dir := t.TempDir()
	writeArtboardFile(t, dir, "Main.dc.html")
	writeFile(t, filepath.Join(dir, "meta.json"), `{"active":true}`) // no "session" field at all
	g := readArtboardGroup(dir, time.Now())
	if g == nil {
		t.Fatalf("expected a group even with no session field")
	}
	html := artboardsRenderGroup(*g)
	if !strings.Contains(html, filepath.Base(dir)) {
		t.Fatalf("expected the rendered identity line to fall back to the folder name, got: %s", html)
	}
}

func TestReadArtboardGroupsWorkspaceClusterAndNewestFirst(t *testing.T) {
	root := t.TempDir()
	now := time.Now()

	mk := func(name, workspace string, age time.Duration) {
		dir := filepath.Join(root, name)
		if err := os.MkdirAll(dir, 0755); err != nil {
			t.Fatalf("MkdirAll: %v", err)
		}
		writeArtboardFile(t, dir, "Main.dc.html")
		setMtime(t, filepath.Join(dir, "Main.dc.html"), now.Add(-age))
		writeFile(t, filepath.Join(dir, "meta.json"), `{"active":true,"workspace":"`+workspace+`"}`)
	}
	mk("b-older", "workspace-1", 2*time.Hour)
	mk("a-newer", "workspace-1", time.Hour)
	mk("c-noworkspace", "", time.Minute)

	groups := readArtboardGroups(root)
	if len(groups) != 3 {
		t.Fatalf("expected 3 groups, got %d", len(groups))
	}
	// workspace-1 groups cluster together (sorted before the empty-workspace
	// group, which sinks to the end like "~"), newest first inside the cluster.
	if groups[0].Name != "a-newer" || groups[1].Name != "b-older" || groups[2].Name != "c-noworkspace" {
		names := []string{groups[0].Name, groups[1].Name, groups[2].Name}
		t.Fatalf("unexpected group order: %v", names)
	}
}

func TestArtboardsRenderRowUsesPillsForTagsPlainTextForState(t *testing.T) {
	group := artboardGroup{
		Name: "g", Session: "sess", Repo: "domux", Workspace: "workspace-1",
		Rows: []artboardRow{{Path: "/tmp/Main.dc.html", Title: "Main", State: "agreed"}},
	}
	groupHTML := artboardsRenderGroup(group)
	if !strings.Contains(groupHTML, `class="tag pill"`) || !strings.Contains(groupHTML, `class="tag ws pill"`) {
		t.Fatalf("expected repo/workspace tags to render as pills, got: %s", groupHTML)
	}
	rowHTML := artboardsRenderRow(group, group.Rows[0])
	if strings.Contains(rowHTML, "pill") {
		t.Fatalf("expected the state token to render as plain text, not a pill, got: %s", rowHTML)
	}
	if !strings.Contains(rowHTML, `class="tok st-agreed"`) {
		t.Fatalf("expected the state token class, got: %s", rowHTML)
	}
}
