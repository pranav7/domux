"""Reference model of the terminal theme before the kind, band and line floors.

Not domux code. The constants here are the design's: FLOOR, GROUND_FLOOR, STEP, MIN_CHROMA and
the hue windows are the numbers decision record 0042 names, and guard.rs in
crates/domux-core/src/theme defines the same ones. terminal_theme4.py extends this model.
"""
from fractions import Fraction as F
import os
import sys

# lib sits beside this file, wherever it is run from.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib import hx, h, cr, oklch, lum, THEMES

FLOOR = 3.0
GROUND_FLOOR = 1.05
STEP = F(1, 18)
MIN_CHROMA = 0.03
RED_WINDOW = (345.0, 40.0)     # hue >= 345 or hue <= 40
GREEN_WINDOW = (115.0, 170.0)  # 115 <= hue <= 170

def rdiv(n, d):
    q, r = divmod(abs(n), d)
    if 2 * r >= d:
        q += 1
    return q if n >= 0 else -q

def mix(a, b, t):
    t = F(t)
    return tuple(rdiv(a[i] * t.denominator + (b[i] - a[i]) * t.numerator, t.denominator) for i in range(3))

BLACK = (0, 0, 0)
WHITE = (255, 255, 255)

ROLES = [
    'overlay_background', 'top_bar_background', 'toast_background', 'fill',
    'sidebar_background', 'tab_row_background',
    'rule', 'separator', 'border',
    'text', 'soft_text', 'dim_text', 'faint_text', 'on_accent', 'on_pill',
    'accent', 'hint_key', 'workspace_name', 'branch', 'pr_open', 'pr_merged', 'pr_closed',
    'pill_ok', 'pill_error', 'config_error', 'question', 'waiting_dot',
    'stay_awake_dot_on', 'stay_awake_dot_off', 'recap', 'recap_seen',
]

DOMUX = dict(
    overlay_background='#1e1e2e', top_bar_background='#181825', toast_background='#181825',
    fill='#313244', sidebar_background='default', tab_row_background='default',
    rule='#313244', separator='#45475a', border='#585b70',
    text='#cdd6f4', soft_text='#a6adc8', dim_text='#7f849c', faint_text='#6c7086',
    on_accent='#1e1e2e', on_pill='#1e1e2e',
    accent='#cba6f7', hint_key='#89b4fa', workspace_name='#93e2d5', branch='#e3b4d8',
    pr_open='#a6e3a1', pr_merged='#cba6f7', pr_closed='#f38ba8',
    pill_ok='#a6e3a1', pill_error='#f38ba8', config_error='#f38ba8', question='#f38ba8',
    waiting_dot='#f38ba8', stay_awake_dot_on='#a6e3a1', stay_awake_dot_off='#585b70',
    recap='#ddcaf7', recap_seen='#a6adc8',
)

TERMINAL = dict(
    overlay_background='background', top_bar_background='shade 1/5', toast_background='shade 1/5',
    fill='blend 1/9', sidebar_background='default', tab_row_background='default',
    rule='blend 1/9', separator='blend 2/9', border='blend 3/9',
    text='foreground', soft_text='blend 7/9', dim_text='blend 5/9', faint_text='blend 4/9',
    on_accent='background', on_pill='background',
    accent='palette 4', hint_key='palette 6', workspace_name='palette 6', branch='palette 5',
    pr_open='palette 2', pr_merged='palette 4', pr_closed='palette 1',
    pill_ok='palette 2', pill_error='palette 1', config_error='palette 1', question='palette 1',
    waiting_dot='palette 1', stay_awake_dot_on='palette 2', stay_awake_dot_off='blend 3/9',
    recap='foreground', recap_seen='blend 7/9',
)

GROUNDS = ['overlay_background', 'top_bar_background', 'toast_background', 'fill',
           'sidebar_background', 'tab_row_background']
MOVABLE_GROUNDS = ['top_bar_background', 'toast_background', 'fill',
                   'sidebar_background', 'tab_row_background']
TEXT_TIERS = ['text', 'soft_text', 'dim_text', 'faint_text', 'recap', 'recap_seen']
TIER_GROUNDS = ['overlay_background', 'top_bar_background', 'toast_background',
                'sidebar_background', 'tab_row_background']
COLOUR_GROUNDS = dict(
    accent=['overlay_background', 'top_bar_background', 'sidebar_background', 'tab_row_background'],
    hint_key=['overlay_background', 'top_bar_background', 'sidebar_background', 'tab_row_background'],
    workspace_name=['overlay_background', 'fill', 'sidebar_background'],
    branch=['overlay_background', 'sidebar_background'],
    pr_open=['overlay_background', 'sidebar_background'],
    pr_merged=['overlay_background', 'sidebar_background'],
    pr_closed=['overlay_background', 'sidebar_background'],
    pill_ok=['overlay_background', 'sidebar_background'],
    pill_error=['overlay_background', 'sidebar_background'],
    config_error=['overlay_background', 'top_bar_background', 'tab_row_background'],
    question=['overlay_background'],
    waiting_dot=['overlay_background', 'fill', 'sidebar_background'],
    stay_awake_dot_on=['overlay_background', 'top_bar_background', 'tab_row_background'],
)
HUE = dict(pr_open='green', pill_ok='green', stay_awake_dot_on='green',
           pr_closed='red', pill_error='red', config_error='red', question='red', waiting_dot='red')

def hue_ok(c, cls):
    L, C, H = oklch(c)
    if C < MIN_CHROMA:
        return False
    if cls == 'red':
        return H >= RED_WINDOW[0] or H <= RED_WINDOW[1]
    return GREEN_WINDOW[0] <= H <= GREEN_WINDOW[1]

def parse(v):
    if v.startswith('#'):
        return ('hex', hx(v))
    if v in ('background', 'foreground', 'default'):
        return (v,)
    kw, arg = v.split()
    if kw == 'palette':
        return ('palette', int(arg))
    n, d = arg.split('/')
    return (kw, F(int(n), int(d)))

def usable(bg, fg):
    return bg is not None and fg is not None and cr(bg, fg) >= FLOOR

def evaluate(form, bg, fg, pal):
    """The colour a form names, or None when the answers it needs are missing."""
    k = form[0]
    if k == 'hex':
        return form[1]
    if not usable(bg, fg):
        return None
    if k in ('background', 'default'):
        return bg
    if k == 'foreground':
        return fg
    if k == 'blend':
        return mix(bg, fg, form[1])
    if k == 'shade':
        far = BLACK if lum(bg) < lum(fg) else WHITE
        return mix(bg, far, form[1])
    if k == 'palette':
        return pal[form[1]] if pal and pal[form[1]] is not None else None

def least_steps(colours, target, grounds, floor):
    """Least k in 0..18 so that every colour moved k/18 toward target meets floor on every ground."""
    for k in range(19):
        if all(cr(mix(c, target, STEP * k), g) >= floor for c in colours for g in grounds):
            return k
    return 18

def paint(chain, bg, fg, pal):
    """chain: list of layers (dicts), named theme first, domux last."""
    val, form, note, layer_of = {}, {}, {}, {}
    root = len(chain) - 1

    def resolve(role, start=0):
        for i, layer in enumerate(chain[start:], start):
            if role not in layer:
                continue
            f = parse(layer[role])
            if f[0] == 'default':
                # Drawn as the terminal's own ground. The guards never look at it.
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

    def considered(names):
        # A `default` ground counts when the background was answered; it never turns guards on.
        return [n for n in names if val[n] is not None]

    def movable(r, names):
        if from_terminal(r):
            return True
        return form[r][0] == 'hex' and layer_of[r] == root and any(from_terminal(g) for g in considered(names))

    # 1. Grounds taken from the terminal keep a visible step from the overlay background.
    for g in ['top_bar_background', 'toast_background', 'fill']:
        if not from_terminal(g) or g == 'overlay_background':
            continue
        k = least_steps([val[g]], text, [over], GROUND_FLOOR)
        if k:
            val[g] = mix(val[g], text, STEP * k)
            note[g] += f'G{k}'

    tier_grounds = [val[n] for n in considered(TIER_GROUNDS)]

    # 2. Text tiers written as a blend move together, toward the far end.
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

    # 3. Colours move on their own toward the far end; red and green are hue tested again.
    for r, names in COLOUR_GROUNDS.items():
        while True:
            if not movable(r, names):
                break
            gs = [val[n] for n in considered(names)]
            kk = least_steps([val[r]], far, gs, FLOOR)
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

def terminal_chain():
    return [TERMINAL, DOMUX]

def answers(name):
    t = THEMES[name]
    return hx(t['background']), hx(t['foreground']), [hx(c) for c in t['palette']]

DRAWN_ON = dict(TIER_GROUNDS=TIER_GROUNDS)

def check_invariants(name, val, note):
    bg = val['overlay_background']
    problems = []
    for r in TEXT_TIERS:
        for g in TIER_GROUNDS:
            if cr(val[r], val[g]) < FLOOR - 1e-9:
                problems.append(f'{r} on {g} {cr(val[r], val[g]):.2f}')
    for r, gs in COLOUR_GROUNDS.items():
        for g in gs:
            if cr(val[r], val[g]) < FLOOR - 1e-9:
                problems.append(f'{r} on {g} {cr(val[r], val[g]):.2f}')
    for r, cls in HUE.items():
        if not hue_ok(val[r], cls):
            problems.append(f'{r} not {cls}: {h(val[r])} {oklch(val[r])}')
    order = [lum(val[r]) for r in ['faint_text', 'dim_text', 'soft_text', 'text']]
    dark = lum(bg) < lum(val['text'])
    if dark and order != sorted(order):
        problems.append('tiers out of order')
    if not dark and order != sorted(order, reverse=True):
        problems.append('tiers out of order')
    for g in ['top_bar_background', 'toast_background', 'fill']:
        if cr(val[g], bg) < GROUND_FLOOR - 1e-9:
            problems.append(f'{g} too close to the overlay')
    return problems

if __name__ == '__main__':
    mode = sys.argv[1] if len(sys.argv) > 1 else 'five'
    if mode == 'five':
        names = sys.argv[2:] or ['ristretto', 'catppuccin', 'catppuccin-latte', 'flexoki-light', 'white']
        data = {n: paint(terminal_chain(), *answers(n)) for n in names}
        print('| role | ' + ' | '.join(names) + ' |')
        print('|---|' + '---|' * len(names))
        for role in ROLES:
            cells = []
            for n in names:
                val, form, note = data[n]
                c = val[role]
                s = f"`{h(c)}`"
                if role not in ('overlay_background', 'on_accent', 'on_pill', 'sidebar_background', 'tab_row_background'):
                    s += f" {cr(c, val['overlay_background']):.2f}"
                if note[role]:
                    s += f" {note[role]}"
                cells.append(s)
            print(f"| `{role}` | " + ' | '.join(cells) + ' |')
    elif mode == 'all':
        for n in THEMES:
            val, form, note = paint(terminal_chain(), *answers(n))
            print(n, check_invariants(n, val, note) or 'ok')
