package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestMarketplaceManifestIsValid(t *testing.T) {
	data, err := os.ReadFile(filepath.Join(".claude-plugin", "marketplace.json"))
	if err != nil {
		t.Fatalf("read marketplace.json: %v", err)
	}
	var m struct {
		Name                                 string `json:"name"`
		AllowCrossMarketplaceDependenciesOn []string `json:"allowCrossMarketplaceDependenciesOn"`
		Plugins []struct {
			Name        string `json:"name"`
			Source      string `json:"source"`
			Description string `json:"description"`
		} `json:"plugins"`
	}
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("parse marketplace.json: %v", err)
	}
	if m.Name != "domux" {
		t.Fatalf("marketplace name = %q, want \"domux\"", m.Name)
	}
	if len(m.Plugins) == 0 {
		t.Fatalf("marketplace has no plugins")
	}
	foundCross := false
	for _, dep := range m.AllowCrossMarketplaceDependenciesOn {
		if dep == "claude-plugins-official" {
			foundCross = true
		}
	}
	if !foundCross {
		t.Fatalf("allowCrossMarketplaceDependenciesOn must include 'claude-plugins-official'; got %v", m.AllowCrossMarketplaceDependenciesOn)
	}
	for _, p := range m.Plugins {
		if p.Name == "" || p.Source == "" {
			t.Fatalf("plugin entry incomplete: %+v", p)
		}
		// Source must resolve to an existing directory.
		if !dirExists(p.Source) {
			t.Fatalf("plugin source %q does not exist", p.Source)
		}
	}
}

func TestPluginManifestIsValid(t *testing.T) {
	path := filepath.Join("plugins", "implement-pipeline", ".claude-plugin", "plugin.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read plugin.json: %v", err)
	}
	var p struct {
		Name        string `json:"name"`
		Version     string `json:"version"`
		Description string `json:"description"`
		Dependencies []struct {
			Name        string `json:"name"`
			Marketplace string `json:"marketplace"`
		} `json:"dependencies"`
	}
	if err := json.Unmarshal(data, &p); err != nil {
		t.Fatalf("parse plugin.json: %v", err)
	}
	if p.Name != "implement-pipeline" {
		t.Fatalf("plugin name = %q, want \"implement-pipeline\"", p.Name)
	}
	if p.Version == "" {
		t.Fatalf("plugin version is empty")
	}
	wantDeps := map[string]bool{
		"superpowers":     false,
		"frontend-design": false,
		"typescript-lsp":  false,
		"pyright-lsp":     false,
	}
	for _, dep := range p.Dependencies {
		if dep.Marketplace != "claude-plugins-official" {
			t.Fatalf("dependency %q has marketplace %q, want \"claude-plugins-official\"", dep.Name, dep.Marketplace)
		}
		if _, ok := wantDeps[dep.Name]; ok {
			wantDeps[dep.Name] = true
		}
	}
	for name, found := range wantDeps {
		if !found {
			t.Fatalf("plugin manifest missing dependency on %q", name)
		}
	}
}

func TestPluginSkillsHaveRequiredFrontmatter(t *testing.T) {
	skills := []string{
		"plugins/implement-pipeline/skills/implement-workflow/SKILL.md",
	}
	for _, s := range skills {
		data, err := os.ReadFile(s)
		if err != nil {
			t.Fatalf("read %s: %v", s, err)
		}
		text := string(data)
		if len(text) < 10 || text[:4] != "---\n" {
			t.Fatalf("%s: missing frontmatter", s)
		}
		// Cheap sanity check: name: and description: present in the head.
		head := text
		if len(head) > 500 {
			head = head[:500]
		}
		if !contains(head, "name:") || !contains(head, "description:") {
			t.Fatalf("%s: frontmatter must declare name and description", s)
		}
	}
}

func contains(haystack, needle string) bool {
	for i := 0; i+len(needle) <= len(haystack); i++ {
		if haystack[i:i+len(needle)] == needle {
			return true
		}
	}
	return false
}
