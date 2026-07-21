package main

import (
	"strings"
	"testing"
	"unicode/utf8"
)

func TestRenderBarWidthAndFill(t *testing.T) {
	bar := renderBar(50, 20)
	if utf8.RuneCountInString(bar) != 20 {
		t.Fatalf("bar rune count = %d, want 20 (%q)", utf8.RuneCountInString(bar), bar)
	}
	filled := strings.Count(bar, "━")
	empty := strings.Count(bar, "╌")
	if filled != 10 || empty != 10 {
		t.Fatalf("filled=%d empty=%d, want 10/10", filled, empty)
	}
}

func TestRenderBarBoundaries(t *testing.T) {
	if strings.Count(renderBar(0, 20), "━") != 0 {
		t.Fatalf("0%% should have no filled cells")
	}
	if strings.Count(renderBar(100, 20), "━") != 20 {
		t.Fatalf("100%% should fill all cells")
	}
	if strings.Count(renderBar(150, 20), "━") != 20 {
		t.Fatalf("over 100%% must clamp to full")
	}
	if utf8.RuneCountInString(renderBar(50, 0)) != 1 {
		t.Fatalf("width<1 should coerce to a 1-cell bar")
	}
}

func TestBarColorThresholds(t *testing.T) {
	if barColor(69) != green {
		t.Fatalf("69%% should be green")
	}
	if barColor(70) != yellow {
		t.Fatalf("70%% should be yellow")
	}
	if barColor(89) != yellow {
		t.Fatalf("89%% should be yellow")
	}
	if barColor(90) != red {
		t.Fatalf("90%% should be red")
	}
}
