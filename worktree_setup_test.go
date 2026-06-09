package main

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestParseWorktreeConf(t *testing.T) {
	in := `# bring local files across
link CLAUDE.local.md

copy .env
run npm install
bogus whatever
link    .vscode/settings.json
copy
`
	dirs, warnings := parseWorktreeConf(strings.NewReader(in))

	want := []setupDirective{
		{Verb: "link", Arg: "CLAUDE.local.md"},
		{Verb: "copy", Arg: ".env"},
		{Verb: "run", Arg: "npm install"},
		{Verb: "link", Arg: ".vscode/settings.json"},
	}
	if !reflect.DeepEqual(dirs, want) {
		t.Fatalf("directives = %#v, want %#v", dirs, want)
	}
	// "bogus whatever" (unknown verb) and "copy" (missing arg) → 2 warnings.
	if len(warnings) != 2 {
		t.Fatalf("warnings = %#v, want 2", warnings)
	}
}

func writeFileMode(t *testing.T, path string, data string, mode os.FileMode) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(data), mode); err != nil {
		t.Fatal(err)
	}
}

func TestApplyWorktreeSetupLinkCopyRun(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	writeFileMode(t, filepath.Join(main, "CLAUDE.local.md"), "hi", 0644)
	writeFileMode(t, filepath.Join(main, ".vscode", "settings.json"), "{}", 0644)
	writeFileMode(t, filepath.Join(main, ".env"), "X=1", 0600)
	if err := os.MkdirAll(filepath.Join(main, "assets"), 0755); err != nil {
		t.Fatal(err)
	}

	var ran []string
	run := func(cmd string) error { ran = append(ran, cmd); return nil }

	dirs := []setupDirective{
		{Verb: "link", Arg: "CLAUDE.local.md"},
		{Verb: "link", Arg: ".vscode/settings.json"},
		{Verb: "link", Arg: "assets"},
		{Verb: "copy", Arg: ".env"},
		{Verb: "run", Arg: "echo a"},
		{Verb: "run", Arg: "echo b"},
	}
	results := applyWorktreeSetup(main, wt, dirs, run)

	// link → symlink pointing at the absolute path in main.
	lp := filepath.Join(wt, "CLAUDE.local.md")
	fi, err := os.Lstat(lp)
	if err != nil || fi.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("CLAUDE.local.md not a symlink: %v %v", fi, err)
	}
	if tgt, _ := os.Readlink(lp); tgt != filepath.Join(main, "CLAUDE.local.md") {
		t.Fatalf("symlink target = %q", tgt)
	}
	// nested link: parent dir created in worktree.
	if _, err := os.Lstat(filepath.Join(wt, ".vscode", "settings.json")); err != nil {
		t.Fatalf("nested link missing: %v", err)
	}
	// dir link works.
	if fi, err := os.Lstat(filepath.Join(wt, "assets")); err != nil || fi.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("assets not a symlink: %v %v", fi, err)
	}
	// copy → independent file, mode preserved, NOT a symlink.
	cp := filepath.Join(wt, ".env")
	cfi, err := os.Lstat(cp)
	if err != nil || cfi.Mode()&os.ModeSymlink != 0 {
		t.Fatalf(".env should be a regular file: %v %v", cfi, err)
	}
	if cfi.Mode().Perm() != 0600 {
		t.Fatalf(".env mode = %v, want 0600", cfi.Mode().Perm())
	}
	if b, _ := os.ReadFile(cp); string(b) != "X=1" {
		t.Fatalf(".env content = %q", b)
	}
	// run dispatched in conf order.
	if !reflect.DeepEqual(ran, []string{"echo a", "echo b"}) {
		t.Fatalf("ran = %#v", ran)
	}
	// every directive reported OK.
	for _, r := range results {
		if !r.OK {
			t.Fatalf("directive not OK: %#v", r)
		}
	}
}

func TestApplyWorktreeSetupMissingSourceIsNonFatal(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	run := func(string) error { return nil }
	results := applyWorktreeSetup(main, wt,
		[]setupDirective{{Verb: "link", Arg: "nope.md"}}, run)
	if len(results) != 1 || results[0].OK {
		t.Fatalf("expected one not-OK result, got %#v", results)
	}
	if _, err := os.Lstat(filepath.Join(wt, "nope.md")); !os.IsNotExist(err) {
		t.Fatalf("nothing should have been created, err=%v", err)
	}
}

func TestApplyWorktreeSetupIdempotent(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	writeFileMode(t, filepath.Join(main, "CLAUDE.local.md"), "hi", 0644)
	writeFileMode(t, filepath.Join(main, ".env"), "X=1", 0644)
	dirs := []setupDirective{
		{Verb: "link", Arg: "CLAUDE.local.md"},
		{Verb: "copy", Arg: ".env"},
	}
	run := func(string) error { return nil }
	applyWorktreeSetup(main, wt, dirs, run)
	results := applyWorktreeSetup(main, wt, dirs, run) // second run must not error
	for _, r := range results {
		if !r.OK {
			t.Fatalf("re-run not OK: %#v", r)
		}
	}
	if tgt, _ := os.Readlink(filepath.Join(wt, "CLAUDE.local.md")); tgt != filepath.Join(main, "CLAUDE.local.md") {
		t.Fatalf("symlink wrong after re-run: %q", tgt)
	}
}

func TestSummarizeSetup(t *testing.T) {
	results := []setupResult{
		{Verb: "link", OK: true},
		{Verb: "link", OK: true},
		{Verb: "copy", OK: true},
		{Verb: "run", OK: true},
		{Verb: "link", Arg: "x", OK: false, Note: "source missing: x"},
	}
	got := summarizeSetup(results)
	if got != "linked 2, copied 1, ran 1, 1 skipped" {
		t.Fatalf("summary = %q", got)
	}
	if summarizeSetup(nil) != "" {
		t.Fatalf("empty summary should be blank")
	}
}

func TestRunWorktreeSetupNoConf(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	results, err := runWorktreeSetup(main, wt, func(string) error { return nil })
	if err != nil {
		t.Fatalf("err = %v", err)
	}
	if results != nil {
		t.Fatalf("expected nil results when no conf, got %#v", results)
	}
}

func TestRunWorktreeSetupAppliesConf(t *testing.T) {
	main := t.TempDir()
	wt := t.TempDir()
	writeFileMode(t, filepath.Join(main, "CLAUDE.local.md"), "hi", 0644)
	writeFileMode(t, filepath.Join(main, ".domux", worktreeConfName),
		"link CLAUDE.local.md\nrun echo hi\n", 0644)
	var ran []string
	results, err := runWorktreeSetup(main, wt, func(c string) error { ran = append(ran, c); return nil })
	if err != nil {
		t.Fatalf("err = %v", err)
	}
	if _, err := os.Lstat(filepath.Join(wt, "CLAUDE.local.md")); err != nil {
		t.Fatalf("link not applied: %v", err)
	}
	if len(ran) != 1 || ran[0] != "echo hi" {
		t.Fatalf("ran = %#v", ran)
	}
	if len(results) != 2 {
		t.Fatalf("results = %#v", results)
	}
}
