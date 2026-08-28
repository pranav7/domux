package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestArtboardsSlug(t *testing.T) {
	cases := map[string]string{
		"harriet-feedback":          "harriet-feedback",
		"Multi-column match (PR 1)": "multi-column-match-pr-1",
		"  leading/trailing !! ":    "leading-trailing",
		"":                          "untitled",
		"!!!":                       "untitled",
		strings.Repeat("a", 60):     strings.Repeat("a", 40),
	}
	for in, want := range cases {
		if got := artboardsSlug(in); got != want {
			t.Errorf("artboardsSlug(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestArtboardsGroupKeyDeterministicAndVerified(t *testing.T) {
	key := artboardsGroupKey("harriet-feedback", "Multi-column match (PR 1)")
	want := "harriet-feedback--multi-column-match-pr-1--833decff" // independently verified via `shasum -a 1`
	if key != want {
		t.Fatalf("artboardsGroupKey = %q, want %q", key, want)
	}
	if again := artboardsGroupKey("harriet-feedback", "Multi-column match (PR 1)"); again != key {
		t.Fatalf("artboardsGroupKey is not deterministic: %q != %q", again, key)
	}
}

func TestArtboardsGroupKeyDiffersOnSlugCollision(t *testing.T) {
	a := artboardsGroupKey("sess", "foo!!!")
	b := artboardsGroupKey("sess", "foo???")
	if a == b {
		t.Fatalf("expected different keys for titles that slugify identically, got %q for both", a)
	}
}

func TestReadArtboardsMetaMissingFile(t *testing.T) {
	meta := readArtboardsMeta(t.TempDir())
	if !meta.Active {
		t.Fatalf("expected a missing meta.json to default Active=true")
	}
	if meta.Session != "" || len(meta.Artboards) != 0 {
		t.Fatalf("expected zero-value meta, got %+v", meta)
	}
}

func TestReadArtboardsMetaMalformedJSON(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "meta.json"), "{not json")
	meta := readArtboardsMeta(dir)
	if !meta.Active {
		t.Fatalf("expected malformed JSON to degrade to the zero value, got %+v", meta)
	}
}

func TestReadArtboardsMetaNonObjectJSON(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "meta.json"), `["not", "an", "object"]`)
	meta := readArtboardsMeta(dir)
	if !meta.Active {
		t.Fatalf("expected non-object JSON to degrade to the zero value, got %+v", meta)
	}
}

func TestReadArtboardsMetaWrongTypedFieldsDegradeIndividually(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "meta.json"), `{
		"session": "harriet-feedback",
		"active": "yes",
		"artboards": {"not": "a list"}
	}`)
	meta := readArtboardsMeta(dir)
	if meta.Session != "harriet-feedback" {
		t.Fatalf("expected a well-typed sibling field to survive, got session=%q", meta.Session)
	}
	if !meta.Active {
		t.Fatalf("expected a wrong-typed active to fall back to true, got %v", meta.Active)
	}
	if len(meta.Artboards) != 0 {
		t.Fatalf("expected a wrong-typed artboards field to fall back to empty, got %+v", meta.Artboards)
	}
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func TestArtboardsInitIdempotentResume(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	root, err := artboardsRoot()
	if err != nil {
		t.Fatalf("artboardsRoot: %v", err)
	}
	groupDir := filepath.Join(root, artboardsGroupKey("sess", "My Title"))
	if err := os.MkdirAll(groupDir, 0755); err != nil {
		t.Fatalf("MkdirAll: %v", err)
	}
	meta := ArtboardsMeta{
		Session: "sess", Title: "My Title", Active: false,
		Artboards: []ArtboardsMetaEntry{{File: "Main.dc.html", State: "agreed"}},
	}
	if err := writeArtboardsMeta(groupDir, meta); err != nil {
		t.Fatalf("writeArtboardsMeta: %v", err)
	}

	// Simulate what artboardsInitCommand does when a group already exists:
	// patch attribution only, leave Active/Artboards untouched.
	resumed := readArtboardsMeta(groupDir)
	resumed.Session = "sess"
	resumed.Workspace = "workspace-2"
	resumed.Repo = "domux"
	resumed.Branch = "main"
	resumed.Title = "My Title"
	if err := writeArtboardsMeta(groupDir, resumed); err != nil {
		t.Fatalf("writeArtboardsMeta (resume): %v", err)
	}

	final := readArtboardsMeta(groupDir)
	if final.Active {
		t.Fatalf("expected Active to stay false across a resume, got true")
	}
	if len(final.Artboards) != 1 || final.Artboards[0].File != "Main.dc.html" {
		t.Fatalf("expected the artboards array to survive a resume untouched, got %+v", final.Artboards)
	}
	if final.Workspace != "workspace-2" {
		t.Fatalf("expected attribution fields to update on resume, got workspace=%q", final.Workspace)
	}
}

func TestArtboardsMigrateSkipsRegularFilesAndDotfiles(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)

	legacyRoot := filepath.Join(homeDir, ".domux", "artboards")
	if err := os.MkdirAll(legacyRoot, 0755); err != nil {
		t.Fatalf("MkdirAll legacyRoot: %v", err)
	}
	// A generated index.html sitting alongside groups, and the dotfiles the
	// prototype leaves behind — none of these are groups to migrate.
	writeFile(t, filepath.Join(legacyRoot, "index.html"), "<html></html>")
	writeFile(t, filepath.Join(legacyRoot, ".server.log"), "log\n")

	groupDir := filepath.Join(legacyRoot, "harriet-feedback")
	if err := os.MkdirAll(groupDir, 0755); err != nil {
		t.Fatalf("MkdirAll groupDir: %v", err)
	}
	writeFile(t, filepath.Join(groupDir, "meta.json"), `{"session":"harriet-feedback","title":"Multi-column match (PR 1)"}`)

	if err := artboardsMigrateCommand([]string{"--apply"}); err != nil {
		t.Fatalf("artboardsMigrateCommand: %v", err)
	}

	newRoot, err := artboardsRoot()
	if err != nil {
		t.Fatalf("artboardsRoot: %v", err)
	}
	wantKey := "harriet-feedback--multi-column-match-pr-1--833decff"
	if !dirExists(filepath.Join(newRoot, wantKey)) {
		t.Fatalf("expected migrated group at %s", filepath.Join(newRoot, wantKey))
	}
	if fileExists(filepath.Join(newRoot, "index.html")) {
		t.Fatalf("index.html must never be treated as a group to migrate")
	}
	if fileExists(filepath.Join(newRoot, ".server.log")) {
		t.Fatalf(".server.log must never be treated as a group to migrate")
	}
	// The legacy group entry itself must be gone (renamed away), not copied.
	if dirExists(groupDir) {
		t.Fatalf("expected the legacy group to be renamed away, not left behind")
	}
}

func TestArtboardsMigratePreviewDoesNotMove(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	legacyRoot := filepath.Join(homeDir, ".domux", "artboards")
	groupDir := filepath.Join(legacyRoot, "harriet-feedback")
	if err := os.MkdirAll(groupDir, 0755); err != nil {
		t.Fatalf("MkdirAll: %v", err)
	}
	writeFile(t, filepath.Join(groupDir, "meta.json"), `{"session":"harriet-feedback","title":"Multi-column match (PR 1)"}`)

	if err := artboardsMigrateCommand(nil); err != nil {
		t.Fatalf("artboardsMigrateCommand (preview): %v", err)
	}
	if !dirExists(groupDir) {
		t.Fatalf("preview-only migrate must not move anything")
	}
}

func TestArtboardsMigrateRefusesExistingTarget(t *testing.T) {
	homeDir := t.TempDir()
	t.Setenv("HOME", homeDir)
	legacyRoot := filepath.Join(homeDir, ".domux", "artboards")
	groupDir := filepath.Join(legacyRoot, "harriet-feedback")
	if err := os.MkdirAll(groupDir, 0755); err != nil {
		t.Fatalf("MkdirAll: %v", err)
	}
	writeFile(t, filepath.Join(groupDir, "meta.json"), `{"session":"harriet-feedback","title":"Multi-column match (PR 1)"}`)

	newRoot, err := artboardsRoot()
	if err != nil {
		t.Fatalf("artboardsRoot: %v", err)
	}
	wantKey := "harriet-feedback--multi-column-match-pr-1--833decff"
	if err := os.MkdirAll(filepath.Join(newRoot, wantKey), 0755); err != nil {
		t.Fatalf("MkdirAll target: %v", err)
	}

	if err := artboardsMigrateCommand([]string{"--apply"}); err == nil {
		t.Fatalf("expected migrate to refuse an existing target")
	}
}
