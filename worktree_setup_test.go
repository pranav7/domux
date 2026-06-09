package main

import (
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
