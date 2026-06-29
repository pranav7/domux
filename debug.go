package main

import (
	"fmt"
	"os"
	"strings"
	"sync"
	"time"
)

// debugLog appends a timestamped line to the domux debug log when the
// DOMUX_DEBUG environment variable is set. It is a no-op otherwise, so the
// hot paths pay nothing in normal runs.
//
// DOMUX_DEBUG controls the destination:
//   - unset or "" / "0" / "false" → disabled
//   - an absolute or relative path → log to that file
//   - any other truthy value ("1", "true", …) → log to the default path,
//     ~/.local/share/domux/debug.log
//
// Logging is best-effort: a path that can't be opened silently disables it
// rather than disrupting the TUI. This exists so tty/attach bugs that only
// surface from a bare shell after a reboot can be reproduced and traced.
func debugLog(format string, args ...any) {
	dest := debugLogDest()
	if dest == "" {
		return
	}
	line := fmt.Sprintf("%s %s\n", time.Now().Format(time.RFC3339), fmt.Sprintf(format, args...))

	debugLogMu.Lock()
	defer debugLogMu.Unlock()
	f, err := os.OpenFile(dest, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return
	}
	defer f.Close()
	_, _ = f.WriteString(line)
}

var debugLogMu sync.Mutex

// debugLogDest resolves DOMUX_DEBUG to a log file path, or "" when disabled.
func debugLogDest() string {
	v := strings.TrimSpace(os.Getenv("DOMUX_DEBUG"))
	switch strings.ToLower(v) {
	case "", "0", "false", "no", "off":
		return ""
	}
	if strings.ContainsAny(v, "/\\") || strings.HasSuffix(strings.ToLower(v), ".log") {
		return v
	}
	path, err := domuxDataDir("debug.log")
	if err != nil {
		return ""
	}
	return path
}
