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
		Name                                string   `json:"name"`
		AllowCrossMarketplaceDependenciesOn []string `json:"allowCrossMarketplaceDependenciesOn"`
		Plugins                             []struct {
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
	wantPlugins := map[string]bool{"domux-start": false, "domux-communicate": false}
	for _, p := range m.Plugins {
		if _, ok := wantPlugins[p.Name]; ok {
			wantPlugins[p.Name] = true
		}
	}
	for name, found := range wantPlugins {
		if !found {
			t.Fatalf("marketplace.json missing plugin %q", name)
		}
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
	path := filepath.Join("plugins", "domux-start", ".claude-plugin", "plugin.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read plugin.json: %v", err)
	}
	var p struct {
		Name        string `json:"name"`
		Version     string `json:"version"`
		Description string `json:"description"`
	}
	if err := json.Unmarshal(data, &p); err != nil {
		t.Fatalf("parse plugin.json: %v", err)
	}
	if p.Name != "domux-start" {
		t.Fatalf("plugin name = %q, want \"domux-start\"", p.Name)
	}
	if p.Version == "" || p.Description == "" {
		t.Fatalf("plugin version/description must be set: %+v", p)
	}
}

func TestPluginSkillsHaveRequiredFrontmatter(t *testing.T) {
	skills := []string{
		"plugins/domux-start/skills/domux-start/SKILL.md",
		"plugins/domux-communicate/skills/domux-communicate/SKILL.md",
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

func TestCommunicatePluginManifestAndSkill(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("plugins", "domux-communicate", ".claude-plugin", "plugin.json"))
	if err != nil {
		t.Fatalf("read plugin.json: %v", err)
	}
	var p struct {
		Name        string `json:"name"`
		Version     string `json:"version"`
		Description string `json:"description"`
	}
	if err := json.Unmarshal(data, &p); err != nil {
		t.Fatalf("parse plugin.json: %v", err)
	}
	if p.Name != "domux-communicate" {
		t.Fatalf("plugin name = %q, want \"domux-communicate\"", p.Name)
	}
	if p.Version == "" || p.Description == "" {
		t.Fatalf("plugin version/description must be set: %+v", p)
	}

	skill := filepath.Join("plugins", "domux-communicate", "skills", "domux-communicate", "SKILL.md")
	sdata, err := os.ReadFile(skill)
	if err != nil {
		t.Fatalf("read SKILL.md: %v", err)
	}
	text := string(sdata)
	if len(text) < 10 || text[:4] != "---\n" {
		t.Fatalf("SKILL.md missing frontmatter")
	}
	head := text
	if len(head) > 500 {
		head = head[:500]
	}
	if !contains(head, "name:") || !contains(head, "description:") {
		t.Fatalf("SKILL.md frontmatter must declare name and description")
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
