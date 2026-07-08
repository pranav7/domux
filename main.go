package main

import (
	"flag"
	"fmt"
	"os"
)

var version = "dev"

func printUsage() {
	fmt.Fprintf(os.Stderr, "█▀▄ █▀█ █▀▄▀█ █ █ ▀▄▀\n")
	fmt.Fprintf(os.Stderr, "█▄▀ █▄█ █ ▀ █ █▄█ █ █  todo · switcher\n\n")
	fmt.Fprintf(os.Stderr, "domux pairs two TUIs for tmux work:\n")
	fmt.Fprintf(os.Stderr, "  todo      per-worktree task list (the \"do\" in domux)\n")
	fmt.Fprintf(os.Stderr, "  switcher  pinned-session picker across all your tmux work\n\n")
	fmt.Fprintf(os.Stderr, "Usage:\n")
	fmt.Fprintf(os.Stderr, "  domux              Open the todo TUI for the current context\n")
	fmt.Fprintf(os.Stderr, "  domux todo         Open the todo TUI (same as bare domux)\n")
	fmt.Fprintf(os.Stderr, "  domux switcher     Open the session switcher\n")
	fmt.Fprintf(os.Stderr, "  domux sessions     Alias for switcher\n\n")
	fmt.Fprintf(os.Stderr, "Sessions:\n")
	fmt.Fprintf(os.Stderr, "  domux start [DIR]  Start or resume a pinned tmux work session\n")
	fmt.Fprintf(os.Stderr, "  domux resume [PROJ]  Recreate saved sessions after a restart (all, or one project)\n")
	fmt.Fprintf(os.Stderr, "  domux adopt [DIR]  Pin the current tmux session to a directory\n")
	fmt.Fprintf(os.Stderr, "  domux attach NAME  Attach/switch to a tmux session\n")
	fmt.Fprintf(os.Stderr, "  domux clear        Reset/free the current tmux workspace\n")
	fmt.Fprintf(os.Stderr, "  domux reset-branch Reset current git branch only\n")
	fmt.Fprintf(os.Stderr, "  domux setup [DIR]  Apply .domux/worktree.conf to a worktree\n")
	fmt.Fprintf(os.Stderr, "  domux server       Toggle current tmux session as running the server\n\n")
	fmt.Fprintf(os.Stderr, "Setup:\n")
	fmt.Fprintf(os.Stderr, "  domux bootstrap    One-shot setup: detect brew/tmux/Claude/Codex/OpenCode, apply hooks\n")
	fmt.Fprintf(os.Stderr, "  domux install      Preview tmux/Claude/Codex/OpenCode/caffeinate integration install\n")
	fmt.Fprintf(os.Stderr, "  domux commands     Utilities popup (caffeinate, …)\n")
	fmt.Fprintf(os.Stderr, "  domux caffeinate   Print/toggle keep-awake (status|on|off|toggle)\n")
	fmt.Fprintf(os.Stderr, "  domux doctor       Check domux/tmux integration health\n\n")
	fmt.Fprintf(os.Stderr, "Agents:\n")
	fmt.Fprintf(os.Stderr, "  domux peek            List running Claude agents across worktrees\n")
	fmt.Fprintf(os.Stderr, "  domux send NAME MSG   Send a message to another worktree's agent\n")
	fmt.Fprintf(os.Stderr, "  domux read NAME       Read another worktree agent's recent output\n\n")
	fmt.Fprintf(os.Stderr, "Status output (for tmux/scripts):\n")
	fmt.Fprintf(os.Stderr, "  domux --path       Print storage path and exit\n")
	fmt.Fprintf(os.Stderr, "  domux --count      Print active task count and exit\n")
	fmt.Fprintf(os.Stderr, "  domux --status     Print top task for tmux status bar\n")
	fmt.Fprintf(os.Stderr, "  domux --list       Print all active tasks\n")
	fmt.Fprintf(os.Stderr, "  domux --version    Print version and exit\n")
	fmt.Fprintf(os.Stderr, "  domux --help       Show this help\n")
}

func main() {
	if len(os.Args) > 1 && os.Args[1] != "" && os.Args[1][0] != '-' {
		if err := runCommand(os.Args[1], os.Args[2:]); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}
		return
	}

	pathFlag := flag.Bool("path", false, "Print the storage path for the current directory and exit")
	countFlag := flag.Bool("count", false, "Print number of active tasks and exit")
	statusFlag := flag.Bool("status", false, "Print top task + count for tmux status bar")
	listFlag := flag.Bool("list", false, "Print all active tasks and exit")
	sessionsFlag := flag.Bool("sessions", false, "Launch session picker TUI")
	versionFlag := flag.Bool("version", false, "Print version and exit")
	flag.Usage = printUsage
	flag.Parse()

	if *versionFlag {
		fmt.Println(version)
		return
	}

	if *sessionsFlag {
		if err := runPicker(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}
		return
	}

	cwd, err := os.Getwd()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: cannot get current directory: %v\n", err)
		os.Exit(1)
	}

	ctx, err := resolveDomuxContext(cwd)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	path := ctx.TodoPath

	if *pathFlag {
		fmt.Println(path)
		return
	}

	if *countFlag {
		list, err := loadList(path)
		if err != nil {
			os.Exit(1)
		}
		fmt.Print(openTaskCount(list))
		return
	}

	if *statusFlag {
		list, err := loadList(path)
		if err != nil {
			os.Exit(1)
		}
		count := openTaskCount(list)
		if count == 0 {
			return
		}
		topItem, ok := focusedOrTopItem(list, ctx.State)
		if !ok {
			return
		}
		top := topItem.Title
		symbol := todoSymbol(topItem)
		if len(top) > 50 {
			top = top[:47] + "..."
		}
		if count > 1 {
			fmt.Printf("%s %s + %d", symbol, top, count-1)
		} else {
			fmt.Printf("%s %s", symbol, top)
		}
		return
	}

	if *listFlag {
		list, err := loadList(path)
		if err != nil {
			os.Exit(1)
		}
		for i, item := range list.Active {
			prefix := "├─"
			if i == len(list.Active)-1 {
				prefix = "└─"
			}
			fmt.Printf("%s %s %s\n", prefix, todoSymbol(item), item.Title)
		}
		return
	}

	if err := runTUI(path); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}
