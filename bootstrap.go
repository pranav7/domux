package main

import (
	"bufio"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type bootstrapEnv struct {
	HasBrew   bool
	HasTmux   bool
	HasClaude bool
	HasCodex  bool
}

type bootstrapStep struct {
	Label string
	Run   func() error
}

func detectBootstrapEnv() bootstrapEnv {
	homeDir, _ := os.UserHomeDir()
	return bootstrapEnv{
		HasBrew:   commandExists("brew"),
		HasTmux:   commandExists("tmux"),
		HasClaude: commandExists("claude") || dirExists(filepath.Join(homeDir, ".claude")),
		HasCodex:  commandExists("codex") || dirExists(filepath.Join(homeDir, ".codex")),
	}
}

func planBootstrap(env bootstrapEnv) []bootstrapStep {
	var steps []bootstrapStep
	if !env.HasTmux && env.HasBrew {
		steps = append(steps, bootstrapStep{
			Label: "brew install tmux (tmux not found)",
			Run:   func() error { return runVisible("brew", "install", "tmux") },
		})
	}
	steps = append(steps, bootstrapStep{
		Label: "write ~/.config/domux/domux.tmux",
		Run:   func() error { return installTmux([]string{"--apply"}) },
	})
	if env.HasClaude {
		steps = append(steps, bootstrapStep{
			Label: "patch ~/.claude/settings.json (Claude Code detected)",
			Run:   func() error { return installClaude([]string{"--apply"}) },
		})
	}
	if env.HasCodex {
		steps = append(steps, bootstrapStep{
			Label: "patch ~/.codex/hooks.json (Codex detected)",
			Run:   func() error { return installCodex([]string{"--apply"}) },
		})
	}
	steps = append(steps, bootstrapStep{
		Label: "register caffeinate (partial — no sudo)",
		Run:   func() error { return installCaffeinatePartial() },
	})
	return steps
}

func bootstrapCommand(args []string) error {
	fs := flag.NewFlagSet("bootstrap", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	yes := fs.Bool("yes", false, "skip confirmation prompt")
	if err := fs.Parse(args); err != nil {
		return err
	}

	env := detectBootstrapEnv()
	steps := planBootstrap(env)

	if !env.HasTmux && !env.HasBrew {
		fmt.Fprintln(os.Stderr, "warning: tmux not found and Homebrew unavailable — install brew (or tmux) before sourcing the generated config.")
	}

	fmt.Println("domux bootstrap will:")
	for _, step := range steps {
		fmt.Printf("  • %s\n", step.Label)
	}
	fmt.Println()

	if !*yes {
		if !confirm(os.Stdin, "Proceed? [y/N]: ") {
			fmt.Println("aborted.")
			return nil
		}
	}

	failures := 0
	for _, step := range steps {
		if err := step.Run(); err != nil {
			fmt.Printf("✗ %s\n    %v\n", step.Label, err)
			failures++
			continue
		}
		fmt.Printf("✓ %s\n", step.Label)
	}

	if failures > 0 {
		return fmt.Errorf("%d step(s) failed", failures)
	}

	fmt.Println()
	fmt.Println("Done. Add `source-file ~/.config/domux/domux.tmux` to ~/.tmux.conf if not already.")
	fmt.Println("Run `domux install caffeinate --full` later if you want lid-close sleep prevention.")
	return nil
}

func confirm(r io.Reader, prompt string) bool {
	if prompt != "" {
		fmt.Print(prompt)
	}
	scanner := bufio.NewScanner(r)
	if !scanner.Scan() {
		return false
	}
	answer := strings.ToLower(strings.TrimSpace(scanner.Text()))
	return answer == "y" || answer == "yes"
}

func runVisible(name string, args ...string) error {
	cmd := exec.Command(name, args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
