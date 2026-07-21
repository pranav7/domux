package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"time"
)

// UsageWindow is one subscription rate-limit window (session / weekly / weekly-Fable).
type UsageWindow struct {
	Label    string
	Percent  int       // 0-100, clamped
	ResetsAt time.Time // zero if the endpoint omitted / sent an unparseable time
}

// UsageSnapshot is the normalized view the TUI renders. It is deliberately
// decoupled from the raw endpoint JSON so schema drift stays in this file.
type UsageSnapshot struct {
	Windows   []UsageWindow
	FetchedAt time.Time
}

// rawUsageWindow / rawUsage encode the (best-guess, confirm-at-verify) shape of
// GET /api/oauth/usage. If the real field names differ, fix them HERE — the
// struct tags are the single source of coupling to the endpoint.
type rawUsageWindow struct {
	Utilization float64 `json:"utilization"` // NOTE: assumed 0-100. If the API sends 0..1 fractions, multiply by 100 in parseUsage.
	ResetsAt    string  `json:"resets_at"`
}

type rawUsage struct {
	FiveHour     *rawUsageWindow `json:"five_hour"`
	SevenDay     *rawUsageWindow `json:"seven_day"`
	SevenDayOpus *rawUsageWindow `json:"seven_day_opus"`
}

// parseUsage converts the raw endpoint body into a normalized snapshot.
// `now` is injected so callers/tests control FetchedAt.
func parseUsage(data []byte, now time.Time) (UsageSnapshot, error) {
	var raw rawUsage
	if err := json.Unmarshal(data, &raw); err != nil {
		return UsageSnapshot{}, fmt.Errorf("cannot parse usage response: %w", err)
	}
	snap := UsageSnapshot{FetchedAt: now}
	add := func(label string, w *rawUsageWindow) {
		if w == nil {
			return
		}
		win := UsageWindow{Label: label, Percent: clampPercent(w.Utilization)}
		if t, err := time.Parse(time.RFC3339, w.ResetsAt); err == nil {
			win.ResetsAt = t
		}
		snap.Windows = append(snap.Windows, win)
	}
	add("Current session", raw.FiveHour)
	add("Current week (all models)", raw.SevenDay)
	add("Current week (Fable)", raw.SevenDayOpus)
	if len(snap.Windows) == 0 {
		return UsageSnapshot{}, errors.New("usage response had no recognized windows")
	}
	return snap, nil
}

func clampPercent(v float64) int {
	p := int(v + 0.5)
	if p < 0 {
		return 0
	}
	if p > 100 {
		return 100
	}
	return p
}
