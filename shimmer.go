package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// shimmerText renders text with a bright band that flows from first letter to
// last, then back.
// frame is a monotonic counter advanced once per spinner tick.
func shimmerText(text string, frame int, dimHex, brightHex string) string {
	if text == "" {
		return ""
	}
	runes := []rune(text)
	n := len(runes)

	// Tail on both sides so the highlight enters and exits cleanly instead of
	// popping in at index 0.
	const tail = 6
	oneWay := n + tail*2
	if oneWay < 12 {
		oneWay = 12
	}

	// Fractional speed → peak slides between chars across frames instead of
	// jumping a whole rune per tick. Smoother glide at 80ms/tick.
	const speed = 1.2
	cycle := oneWay * 2
	phase := math.Mod(float64(frame)*speed, float64(cycle))
	if phase < 0 {
		phase += float64(cycle)
	}
	if phase > float64(oneWay) {
		phase = float64(cycle) - phase
	}
	pos := phase - float64(tail)
	const sigma = 2.2
	// Higher floor → trailing chars stay legible; less "fade to dark".
	const floor = 0.35

	var b strings.Builder
	for i, r := range runes {
		d := float64(i) - pos
		bright := math.Exp(-(d * d) / (2 * sigma * sigma))
		if bright < floor {
			bright = floor
		}
		col := lerpHex(dimHex, brightHex, bright)
		b.WriteString(lipgloss.NewStyle().Foreground(lipgloss.Color(col)).Render(string(r)))
	}
	return b.String()
}

func lerpHex(a, b string, t float64) string {
	ar, ag, ab := parseHexRGB(a)
	br, bg, bb := parseHexRGB(b)
	r := int(float64(ar)*(1-t) + float64(br)*t + 0.5)
	g := int(float64(ag)*(1-t) + float64(bg)*t + 0.5)
	bl := int(float64(ab)*(1-t) + float64(bb)*t + 0.5)
	return fmt.Sprintf("#%02X%02X%02X", clampByte(r), clampByte(g), clampByte(bl))
}

func parseHexRGB(h string) (int, int, int) {
	h = strings.TrimPrefix(h, "#")
	if len(h) != 6 {
		return 0, 0, 0
	}
	r, _ := strconv.ParseInt(h[0:2], 16, 64)
	g, _ := strconv.ParseInt(h[2:4], 16, 64)
	b, _ := strconv.ParseInt(h[4:6], 16, 64)
	return int(r), int(g), int(b)
}

func clampByte(v int) int {
	if v < 0 {
		return 0
	}
	if v > 255 {
		return 255
	}
	return v
}
