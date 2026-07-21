package main

import (
	"testing"
	"time"
)

func TestParseUsageMapsThreeWindows(t *testing.T) {
	body := []byte(`{
		"five_hour":       {"utilization": 15, "resets_at": "2026-07-21T19:29:00Z"},
		"seven_day":       {"utilization": 24, "resets_at": "2026-07-27T19:59:00Z"},
		"seven_day_opus":  {"utilization": 4,  "resets_at": "2026-07-27T19:59:00Z"}
	}`)
	now := time.Date(2026, 7, 21, 12, 0, 0, 0, time.UTC)
	snap, err := parseUsage(body, now)
	if err != nil {
		t.Fatalf("parseUsage: %v", err)
	}
	if !snap.FetchedAt.Equal(now) {
		t.Fatalf("FetchedAt = %v", snap.FetchedAt)
	}
	if len(snap.Windows) != 3 {
		t.Fatalf("windows = %#v", snap.Windows)
	}
	w := snap.Windows[0]
	if w.Label != "Current session" || w.Percent != 15 {
		t.Fatalf("window[0] = %#v", w)
	}
	if snap.Windows[2].Label != "Current week (Fable)" || snap.Windows[2].Percent != 4 {
		t.Fatalf("window[2] = %#v", snap.Windows[2])
	}
	if snap.Windows[0].ResetsAt.IsZero() {
		t.Fatalf("expected parsed reset time")
	}
}

func TestParseUsageSkipsMissingWindows(t *testing.T) {
	body := []byte(`{"five_hour": {"utilization": 50, "resets_at": "2026-07-21T19:29:00Z"}}`)
	snap, err := parseUsage(body, time.Now())
	if err != nil {
		t.Fatalf("parseUsage: %v", err)
	}
	if len(snap.Windows) != 1 || snap.Windows[0].Label != "Current session" {
		t.Fatalf("windows = %#v", snap.Windows)
	}
}

func TestParseUsageErrorsOnGarbage(t *testing.T) {
	if _, err := parseUsage([]byte(`not json`), time.Now()); err == nil {
		t.Fatalf("expected error on malformed JSON")
	}
}

func TestParseUsageErrorsWhenNoWindows(t *testing.T) {
	if _, err := parseUsage([]byte(`{}`), time.Now()); err == nil {
		t.Fatalf("expected error when no recognized windows")
	}
}

func TestClampPercent(t *testing.T) {
	cases := map[float64]int{-5: 0, 0: 0, 14.6: 15, 100: 100, 150: 100}
	for in, want := range cases {
		if got := clampPercent(in); got != want {
			t.Fatalf("clampPercent(%v) = %d, want %d", in, got, want)
		}
	}
}
