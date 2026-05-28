package main

import (
	"bufio"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type Item struct {
	ID         string
	Title      string
	Notes      string
	InProgress bool
	Done       bool
	DoneDate   string // YYYY-MM-DD
}

type List struct {
	Worktree string
	Created  string // YYYY-MM-DD
	Active   []Item
	Archive  []Item
}

func loadList(path string) (*List, error) {
	if _, err := os.Stat(path); os.IsNotExist(err) {
		return &List{
			Created: time.Now().Format("2006-01-02"),
			Active:  []Item{},
			Archive: []Item{},
		}, nil
	}

	f, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("cannot open file: %w", err)
	}
	defer f.Close()

	list := &List{Active: []Item{}, Archive: []Item{}}
	scanner := bufio.NewScanner(f)

	frontmatterDone := false
	inFrontmatter := false
	inTodos := false
	inArchive := false
	var currentItem *Item

	flushItem := func() {
		if currentItem == nil {
			return
		}
		if inArchive {
			list.Archive = append(list.Archive, *currentItem)
		} else {
			list.Active = append(list.Active, *currentItem)
		}
		currentItem = nil
	}

	for scanner.Scan() {
		line := scanner.Text()

		// Frontmatter parsing (only at top of file)
		if !frontmatterDone && line == "---" {
			if !inFrontmatter {
				inFrontmatter = true
			} else {
				inFrontmatter = false
				frontmatterDone = true
			}
			continue
		}
		if inFrontmatter {
			if strings.HasPrefix(line, "worktree:") {
				list.Worktree = strings.TrimSpace(strings.TrimPrefix(line, "worktree:"))
			} else if strings.HasPrefix(line, "created:") {
				list.Created = strings.TrimSpace(strings.TrimPrefix(line, "created:"))
			}
			continue
		}

		// Section headers
		if line == "# TODOs" {
			flushItem()
			inTodos = true
			inArchive = false
			continue
		}
		if line == "## Archive" {
			flushItem()
			inArchive = true
			inTodos = false
			continue
		}

		// Active items
		if inTodos && (strings.HasPrefix(line, "- [ ] ") || strings.HasPrefix(line, "- [~] ") || strings.HasPrefix(line, "- [x] ")) {
			flushItem()
			inProgress := strings.HasPrefix(line, "- [~] ")
			done := strings.HasPrefix(line, "- [x] ")
			title := strings.TrimPrefix(line, "- [ ] ")
			title = strings.TrimPrefix(title, "- [~] ")
			title = strings.TrimPrefix(title, "- [x] ")
			doneDate := ""
			if done {
				doneDate, title = splitDoneDateTitle(title)
			}
			title, id := splitTodoID(title)
			currentItem = &Item{
				ID:         id,
				Title:      title,
				InProgress: inProgress,
				Done:       done,
				DoneDate:   doneDate,
			}
			continue
		}

		// Archive items
		if inArchive && strings.HasPrefix(line, "- [x] ") {
			flushItem()
			rest := strings.TrimPrefix(line, "- [x] ")
			doneDate, title := splitDoneDateTitle(rest)
			id := ""
			title, id = splitTodoID(title)
			currentItem = &Item{
				ID:       id,
				DoneDate: doneDate,
				Title:    title,
				Done:     true,
			}
			continue
		}

		// Notes (indented continuation lines)
		if currentItem != nil && strings.HasPrefix(line, "  ") {
			note := strings.TrimPrefix(line, "  ")
			if currentItem.Notes != "" {
				currentItem.Notes += "\n"
			}
			currentItem.Notes += note
			continue
		}

		// Any non-indented, non-item line ends the current item
		if currentItem != nil {
			flushItem()
		}
	}

	flushItem()

	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("error reading file: %w", err)
	}

	return list, nil
}

func saveList(path string, list *List) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("cannot create directory: %w", err)
	}

	tmpPath := path + ".tmp"
	f, err := os.Create(tmpPath)
	if err != nil {
		return fmt.Errorf("cannot create temp file: %w", err)
	}

	var b strings.Builder
	b.WriteString("---\n")
	if list.Worktree != "" {
		b.WriteString(fmt.Sprintf("worktree: %s\n", list.Worktree))
	}
	b.WriteString(fmt.Sprintf("created: %s\n", list.Created))
	b.WriteString("---\n\n")

	b.WriteString("# TODOs\n\n")
	for i := range list.Active {
		item := &list.Active[i]
		ensureItemID(item)
		marker := " "
		if item.Done {
			marker = "x"
		} else if item.InProgress {
			marker = "~"
		}
		title := item.Title
		if item.Done && item.DoneDate != "" {
			title = item.DoneDate + " — " + title
		}
		b.WriteString(fmt.Sprintf("- [%s] %s%s\n", marker, title, todoIDComment(item.ID)))
		if item.Notes != "" {
			for _, line := range strings.Split(item.Notes, "\n") {
				b.WriteString(fmt.Sprintf("  %s\n", line))
			}
		}
	}

	if len(list.Archive) > 0 {
		b.WriteString("\n## Archive\n\n")
		for i := range list.Archive {
			item := &list.Archive[i]
			ensureItemID(item)
			if item.DoneDate != "" {
				b.WriteString(fmt.Sprintf("- [x] %s — %s%s\n", item.DoneDate, item.Title, todoIDComment(item.ID)))
			} else {
				b.WriteString(fmt.Sprintf("- [x] %s%s\n", item.Title, todoIDComment(item.ID)))
			}
			if item.Notes != "" {
				for _, line := range strings.Split(item.Notes, "\n") {
					b.WriteString(fmt.Sprintf("  %s\n", line))
				}
			}
		}
	}

	if _, err := f.WriteString(b.String()); err != nil {
		f.Close()
		os.Remove(tmpPath)
		return fmt.Errorf("cannot write to temp file: %w", err)
	}

	if err := f.Close(); err != nil {
		os.Remove(tmpPath)
		return fmt.Errorf("cannot close temp file: %w", err)
	}

	if err := os.Rename(tmpPath, path); err != nil {
		os.Remove(tmpPath)
		return fmt.Errorf("cannot rename temp file: %w", err)
	}

	return nil
}

func splitTodoID(title string) (string, string) {
	const prefix = "<!-- domux:id="
	const suffix = " -->"
	idx := strings.LastIndex(title, prefix)
	if idx < 0 || !strings.HasSuffix(title, suffix) {
		return strings.TrimSpace(title), ""
	}
	id := strings.TrimSuffix(strings.TrimPrefix(title[idx:], prefix), suffix)
	return strings.TrimSpace(title[:idx]), strings.TrimSpace(id)
}

func splitDoneDateTitle(rest string) (string, string) {
	parts := strings.SplitN(rest, " — ", 2)
	if len(parts) != 2 {
		return "", rest
	}
	return strings.TrimSpace(parts[0]), strings.TrimSpace(parts[1])
}

func openTaskCount(list *List) int {
	if list == nil {
		return 0
	}
	count := 0
	for _, item := range list.Active {
		if !item.Done {
			count++
		}
	}
	return count
}

func todoSymbol(item Item) string {
	switch {
	case item.Done:
		return "✓"
	case item.InProgress:
		return "●"
	default:
		return "○"
	}
}

func todoIDComment(id string) string {
	if id == "" {
		return ""
	}
	return " <!-- domux:id=" + id + " -->"
}

func ensureItemID(item *Item) {
	if item == nil || item.ID != "" {
		return
	}
	item.ID = newTodoID()
}

func newTodoID() string {
	var b [8]byte
	if _, err := rand.Read(b[:]); err == nil {
		return hex.EncodeToString(b[:])
	}
	return fmt.Sprintf("%d", time.Now().UnixNano())
}
