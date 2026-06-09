package main

import (
	"bufio"
	"fmt"
	"io"
	"strings"
)

const worktreeConfName = "worktree.conf"

// setupDirective is one parsed line of .domux/worktree.conf.
type setupDirective struct {
	Verb string // link | copy | run
	Arg  string // path (link/copy) or command (run); rest-of-line, trimmed
}

// parseWorktreeConf reads line-oriented directives. Blank lines and #-comments
// are ignored. Unknown verbs and empty args become warnings (skipped), so an
// older domux binary degrades gracefully against a newer conf.
func parseWorktreeConf(r io.Reader) (directives []setupDirective, warnings []string) {
	sc := bufio.NewScanner(r)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		verb, rest, _ := strings.Cut(line, " ")
		arg := strings.TrimSpace(rest)
		switch verb {
		case "link", "copy", "run":
			if arg == "" {
				warnings = append(warnings, fmt.Sprintf("%s: missing argument", verb))
				continue
			}
			directives = append(directives, setupDirective{Verb: verb, Arg: arg})
		default:
			warnings = append(warnings, fmt.Sprintf("unknown directive %q", line))
		}
	}
	return directives, warnings
}
