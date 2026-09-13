"""Reference model of the terminal theme after the author's decisions (decision record 0042).

Extends terminal_theme3.py with the kind colours and band ends held to the text floor under the
terminal theme, and a small floor for the lines and the stay awake dot for "not held". It is not
domux code: the Rust fixture tests in crates/domux-core/src/theme are the reference, and this
model is where their pinned values came from.

    python3 terminal_theme4.py all       one line per Omarchy theme: ok, or what fails
    python3 terminal_theme4.py five      the role table for the five themes the brief named
    python3 terminal_theme4.py summary   the lines, kinds and band ends moved on all 22
"""
from fractions import Fraction as F
import os
import sys

# terminal_theme3 and lib sit beside this file, wherever it is run from.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import terminal_theme3 as t3
from terminal_theme3 import (mix, least_steps, hue_ok, parse, usable, evaluate, BLACK, WHITE, STEP,
                             FLOOR, GROUND_FLOOR, TEXT_TIERS, TIER_GROUNDS, COLOUR_GROUNDS, HUE,
                             TERMINAL, answers)
from lib import cr, lum, h, oklch, THEMES

RULE_FLOOR = 1.25
LINE_FLOOR = 1.4
DOT_OFF_FLOOR = 2.0

KINDS = dict(claude='#de7356', codex='#89b4fa', opencode='#c678b8', compacting='#afafff',
             band_claude_dim='#b85e47', band_claude_bright='#ffc9b0',
             band_codex_dim='#6478a8', band_codex_bright='#c8daff',
             band_opencode_dim='#9f5d93', band_opencode_bright='#f0b5e3',
             band_compacting_dim='#6f6fcf', band_compacting_bright='#d8d8ff')
DOMUX = dict(t3.DOMUX, **KINDS)
ROLES = t3.ROLES + list(KINDS)

KIND_GROUNDS = ['overlay_background', 'sidebar_background']
ALL_FOUR = ['overlay_background', 'sidebar_background', 'top_bar_background', 'tab_row_background']
LINES = dict(
    rule=(RULE_FLOOR, ALL_FOUR),
    separator=(LINE_FLOOR, ALL_FOUR),
    border=(LINE_FLOOR, ALL_FOUR + ['toast_background']),
    stay_awake_dot_off=(DOT_OFF_FLOOR, ['overlay_background', 'top_bar_background', 'tab_row_background']),
)
FLOORED = dict({r: (FLOOR, gs) for r, gs in COLOUR_GROUNDS.items()},
               **{k: (FLOOR, KIND_GROUNDS) for k in KINDS}, **LINES)


def paint(chain, bg, fg, pal, kind_grounds=KIND_GROUNDS):
    val, form, note, layer_of = {}, {}, {}, {}
    root = len(chain) - 1

    def resolve(role, start=0):
        for i, layer in enumerate(chain[start:], start):
            if role not in layer:
                continue
            f = parse(layer[role])
            if f[0] == 'default':
                return (bg if usable(bg, fg) else None), f, i
            c = evaluate(f, bg, fg, pal)
            if c is None:
                continue
            if f[0] == 'palette' and role in HUE and not hue_ok(c, HUE[role]):
                note[role] = note.get(role, '') + 'H'
                continue
            return c, f, i
        raise AssertionError(role)

    for role in ROLES:
        val[role], form[role], layer_of[role] = resolve(role)
        note.setdefault(role, '')

    from_terminal = lambda r: form[r][0] not in ('hex', 'default')
    if not any(from_terminal(r) for r in ROLES):
        return val, form, note

    text = val['text']
    over = val['overlay_background']
    far = WHITE if lum(over) < lum(text) else BLACK
    considered = lambda names: [n for n in names if val[n] is not None]

    def movable(r, names):
        if from_terminal(r):
            return True
        return form[r][0] == 'hex' and layer_of[r] == root and any(from_terminal(g) for g in considered(names))

    for g in ['top_bar_background', 'toast_background', 'fill']:
        if not from_terminal(g):
            continue
        k = least_steps([val[g]], text, [over], GROUND_FLOOR)
        if k:
            val[g] = mix(val[g], text, STEP * k)
            note[g] += f'G{k}'

    tier_grounds = [val[n] for n in considered(TIER_GROUNDS)]
    blends = [r for r in TEXT_TIERS if form[r][0] == 'blend']
    if blends:
        k = least_steps([val[r] for r in blends], far, tier_grounds, FLOOR)
        if k:
            for r in blends:
                val[r] = mix(val[r], far, STEP * k)
                note[r] += f'S{k}'
    for r in TEXT_TIERS:
        if form[r][0] == 'blend' or not movable(r, TIER_GROUNDS):
            continue
        kk = least_steps([val[r]], far, tier_grounds, FLOOR)
        if kk:
            val[r] = mix(val[r], far, STEP * kk)
            note[r] += f'P{kk}'

    floored = dict(FLOORED)
    for k in KINDS:
        floored[k] = (FLOOR, kind_grounds)
    for r, (floor, names) in floored.items():
        while True:
            if not movable(r, names):
                break
            gs = [val[n] for n in considered(names)]
            kk = least_steps([val[r]], far, gs, floor)
            moved = mix(val[r], far, STEP * kk)
            if r in HUE and form[r][0] == 'palette' and not hue_ok(moved, HUE[r]):
                note[r] += 'R'
                val[r], form[r], layer_of[r] = resolve(r, layer_of[r] + 1)
                continue
            if kk:
                note[r] += f'P{kk}'
            val[r] = moved
            break
    return val, form, note


def chain():
    return [TERMINAL, DOMUX]


def check(val):
    probs = []
    for r in TEXT_TIERS:
        for g in TIER_GROUNDS:
            if cr(val[r], val[g]) < FLOOR - 1e-9:
                probs.append(f'{r} on {g}')
    for r, (floor, gs) in FLOORED.items():
        for g in gs:
            if cr(val[r], val[g]) < floor - 1e-9:
                probs.append(f'{r} on {g} {cr(val[r], val[g]):.2f}')
    for r, cls in HUE.items():
        if not hue_ok(val[r], cls):
            probs.append(f'{r} not {cls}')
    bg = val['overlay_background']
    order = [lum(val[r]) for r in ['faint_text', 'dim_text', 'soft_text', 'text']]
    dark = lum(bg) < lum(val['text'])
    if order != sorted(order, reverse=not dark):
        probs.append('tiers out of order')
    lines = [cr(val[r], bg) for r in ['rule', 'separator', 'border']]
    if lines != sorted(lines):
        probs.append('lines out of order')
    for g in ['top_bar_background', 'toast_background', 'fill']:
        if cr(val[g], bg) < GROUND_FLOOR - 1e-9:
            probs.append(f'{g} too close')
    return probs


def five(names):
    data = {n: paint(chain(), *answers(n)) for n in names}
    out = ['| role | ' + ' | '.join(names) + ' |', '|---|' + '---|' * len(names)]
    for role in ROLES:
        cells = []
        for n in names:
            val, form, note = data[n]
            s = f"`{h(val[role])}`"
            if role not in ('overlay_background', 'on_accent', 'on_pill', 'sidebar_background', 'tab_row_background'):
                s += f" {cr(val[role], val['overlay_background']):.2f}"
            if note[role]:
                s += f" {note[role]}"
            cells.append(s)
        out.append(f"| `{role}` | " + ' | '.join(cells) + ' |')
    return '\n'.join(out)


def summary():
    out = ['| theme | lines moved | kinds and band ends moved | lowest kind or band contrast on its grounds | lowest line contrast against its floor |',
           '|---|---|---|---|---|']
    for n in THEMES:
        val, form, note = paint(chain(), *answers(n))
        lines = ', '.join(f"{r} +{note[r][1:]}" for r in LINES if note[r]) or 'none'
        kinds = ', '.join(f"{r} +{note[r][1:]}" for r in KINDS if note[r]) or 'none'
        lowk = min(cr(val[k], val[g]) for k in KINDS for g in KIND_GROUNDS)
        lowl = ', '.join(f"{r} {min(cr(val[r], val[g]) for g in gs):.2f}" for r, (fl, gs) in LINES.items())
        out.append(f'| {n} | {lines} | {kinds} | {lowk:.2f} | {lowl} |')
    return '\n'.join(out)


if __name__ == '__main__':
    mode = sys.argv[1] if len(sys.argv) > 1 else 'all'
    if mode == 'all':
        for n in THEMES:
            val, form, note = paint(chain(), *answers(n))
            moved = [f'{r} {note[r]}' for r in list(LINES) + list(KINDS) if note[r]]
            print(n, check(val) or 'ok', '|', ', '.join(moved) or 'nothing moved')
    elif mode == 'five':
        print(five(sys.argv[2:] or ['ristretto', 'catppuccin', 'catppuccin-latte', 'flexoki-light', 'white']))
    elif mode == 'summary':
        print(summary())
    else:
        sys.exit(f'unknown mode {mode}; use all, five or summary')
