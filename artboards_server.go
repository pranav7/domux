package main

import (
	"errors"
	"fmt"
	"log"
	"net/http"
	"os"
	"path"
	"path/filepath"
	"strings"
)

const artboardsPort = 8899

// artboardsAllowedExt is the exact set of file types ever served — smaller
// than the prototype's SimpleHTTPRequestHandler (which served anything).
// meta.json is never on this list: the generator reads it straight off disk
// in-process and it never needs to be fetched over HTTP. filepath.Ext only
// returns the last dot, so ".dc.html" needs no separate entry from ".html".
// No .js/.css either — the artboards format is one self-contained HTML
// document with no external assets beyond a CDN font stylesheet, so neither
// type has a legitimate local file to serve and both are dropped rather than
// left as unused attack surface.
var artboardsAllowedExt = map[string]string{
	".html": "text/html; charset=utf-8",
	".png":  "image/png",
	".jpg":  "image/jpeg",
	".jpeg": "image/jpeg",
	".svg":  "image/svg+xml",
}

var errArtboardsBlocked = errors.New("artboards: path blocked")

// artboardsResolvePath maps "/<group>/<file...>" to a concrete, contained
// file on disk. The group segment is resolved through its own symlink to
// its real target first (that's the whole point of a symlinked group — a
// notes folder or a worktree); the full path is then resolved through
// symlinks again (catching a symlink *inside* the group pointing back out),
// and the result must still be a descendant of the resolved group dir.
// 404 and "blocked" collapse to the same response deliberately — distinguishing
// them would leak which out-of-bounds paths exist.
func artboardsResolvePath(root, urlPath string) (string, error) {
	clean := path.Clean("/" + urlPath) // collapses any literal ".." before it touches the filesystem
	segments := strings.Split(strings.TrimPrefix(clean, "/"), "/")
	if len(segments) == 0 || segments[0] == "" || strings.HasPrefix(segments[0], ".") {
		return "", errArtboardsBlocked
	}

	groupDir := filepath.Join(root, filepath.FromSlash(segments[0]))
	resolvedGroupDir, err := filepath.EvalSymlinks(groupDir)
	if err != nil {
		return "", errArtboardsBlocked
	}

	requested := filepath.Join(resolvedGroupDir, filepath.Join(segments[1:]...))
	resolved, err := filepath.EvalSymlinks(requested)
	if err != nil {
		return "", errArtboardsBlocked
	}

	rel, err := filepath.Rel(resolvedGroupDir, resolved)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return "", errArtboardsBlocked
	}

	ext := strings.ToLower(filepath.Ext(resolved))
	if _, ok := artboardsAllowedExt[ext]; !ok {
		return "", errArtboardsBlocked
	}
	info, err := os.Stat(resolved)
	if err != nil || info.IsDir() {
		return "", errArtboardsBlocked
	}
	return resolved, nil
}

// artboardsMux builds the server's routes. /healthz is fixed and
// side-effect-free (no index regen) so the picker's frequent poll never
// triggers a full walk-every-group-and-rewrite-index.html cycle just to
// answer "are you alive". Every other path is never listed as a directory —
// stricter than the prototype's SimpleHTTPRequestHandler, which lists a
// directory lacking its own index.html.
func artboardsMux(root string) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok\n"))
	})
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		if r.URL.Path == "/" || r.URL.Path == "/index.html" {
			if _, err := regenerateArtboardsIndex(root); err != nil {
				log.Printf("artboards: index regen failed: %v", err) // never fail the request over one bad group
			}
			http.ServeFile(w, r, filepath.Join(root, "index.html"))
			return
		}
		resolved, err := artboardsResolvePath(root, r.URL.Path)
		if err != nil {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Content-Type", artboardsAllowedExt[strings.ToLower(filepath.Ext(resolved))])
		http.ServeFile(w, r, resolved)
	})
	return mux
}

func artboardsServeCommand(args []string) error {
	if len(args) != 0 {
		return fmt.Errorf("serve does not accept arguments")
	}
	root, err := artboardsRoot()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(root, 0755); err != nil {
		return fmt.Errorf("cannot create %s: %w", root, err)
	}
	addr := fmt.Sprintf("127.0.0.1:%d", artboardsPort)
	fmt.Printf("artboards on http://%s/ serving %s\n", addr, root)
	return http.ListenAndServe(addr, artboardsMux(root))
}
