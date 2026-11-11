import os
# gen_window.py <asm name> <start_win id> <W> <H> : widget table for one window from btnemu.py
import json, subprocess, sys
name, wid, W, H = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
def run(*extra):
    return json.loads(subprocess.run(['python3', 'btnemu.py', f'out/asm/{name}.asm', W, H, *extra], capture_output=True, text=True).stdout)
calls = run()
# Tabbed windows: walk once per tab value and keep, for each call site, the tabs it is drawn on.
tabs = {}
for c in calls:
    if c['fn'].startswith('BTN_tab') and isinstance(c['int'][2], str) and isinstance(c['int'][3], int) and c['int'][3] < 64:
        tabs.setdefault(c['int'][2][1:], []).append(c['int'][3])
if not tabs:
    # No tabs: radio buttons (BTN_is_int on one variable) switch sections the same way (Weather).
    for c in calls:
        if c['fn'].startswith('BTN_is_int') and isinstance(c['int'][0], str) and c['int'][0].startswith('&') and isinstance(c['int'][1], int):
            tabs.setdefault(c['int'][0][1:], []).append(c['int'][1])
    tabs = {k: v for k, v in tabs.items() if len(v) >= 3}
TABCOND = {}
if tabs:
    var, values = next(iter(tabs.items()))
    seen = {}
    for v in values:
        for c in run(f'--tab={var}={v}'):
            seen.setdefault(c['pc'], (c, set()))[1].add(v)
    calls = [seen[pc][0] for pc in sorted(seen, key=lambda p: int(p, 16))]
    for pc, (c, vs) in seen.items():
        if len(vs) < len(values):
            TABCOND[pc] = var + '=' + '/'.join(str(v) for v in sorted(vs))
rows = []
def f(v):
    if isinstance(v, float): return '%g' % v
    if v is None: return ''
    return str(v)
def row(kind, x, y, a='', b='', c='', d='', key='', label='', items='', cond=''):
    rows.append('\t'.join(f(v) for v in (kind, x, y, a, b, c, d, key, str(label).replace('\t', ' ').replace('\n', ' '), items, cond)))
def S(v): return v if isinstance(v, str) and v != '?' and not v.startswith('stk') else ''
def lab(v): return S(v).split(' { ')[0]
def key(v, label):
    if isinstance(v, str) and v.startswith('&'): return v[1:]
    if isinstance(v, list) and v and v[0] == 'thisp': return 'this+0x%x' % v[1]
    if isinstance(v, list) and v and v[0] == 'mem': return 'm:' + str(v[1])
    return 'lbl:' + (label or 'anon%d' % len(rows))
tabvars = {c['int'][2][1:] for c in calls if c['fn'].startswith('BTN_tab') and isinstance(c['int'][2], str)}
for c in calls:
    fn, xm, it, st = c['fn'].split('(')[0], c['xmm'], c['int'], c['str']
    cond = TABCOND.get(c['pc'], '')
    num = lambda v: v if isinstance(v, float) else None
    if fn == 'BTN_subwin' and 'rect' in c:
        r = c['rect']; row('subwin', r[0], r[1], r[2], r[3], it[0], label=S(st[1]), cond=cond)
    elif fn == 'BTN_frame2' and num(xm[0]) is not None:
        row('subwin', xm[0], xm[1], xm[2], xm[3], it[0], cond=cond)
    elif num(xm[0]) is None and fn not in ('BTN_tab',):
        continue
    elif fn == 'BTN_bool':
        row('bool', xm[0], xm[1], key=key(it[0], lab(st[1])), label=lab(st[1]), cond=cond)
    elif fn == 'BTN_textlist':
        row('popup', xm[0], xm[1], key=key(it[0], lab(st[2])), label=lab(st[2]), items=S(st[1]), cond=cond)
    elif fn in ('BTN_flt_range', 'BTN_dob_range'):
        row('dob', xm[0], xm[1], it[1], it[2], xm[2], xm[3], key=key(it[0], lab(st[3])), label=lab(st[3]), cond=cond)
    elif fn == 'BTN_int_range':
        row('dob', xm[0], xm[1], it[1], 0, xm[2], xm[3], key=key(it[0], lab(st[2])), label=lab(st[2]), cond=cond)
    elif fn in ('BTN_flt', 'BTN_dob'):
        row('dob', xm[0], xm[1], it[1], it[2], -1e9, 1e9, key=key(it[0], lab(st[4])), label=lab(st[4]), cond=cond)
    elif fn == 'BTN_int':
        row('dob', xm[0], xm[1], it[1], 0, -1e9, 1e9, key=key(it[0], lab(st[3])), label=lab(st[3]), cond=cond)
    elif fn == 'BTN_is_int':
        row('radio', xm[0], xm[1], it[1], key=key(it[0], lab(st[2])), label=lab(st[2]), cond=cond)
    elif fn == 'BTN_action':
        row('action', xm[0], xm[1], xm[2], key='act:' + S(st[2]), label=S(st[2]), cond=cond)
    elif fn == 'BTN_horz_slider':
        lo, hi = num(xm[3]), num(xm[4])
        row('slider', xm[0], xm[1], xm[2], lo if lo is not None else 0.0, hi if hi is not None else 1.0, key=key(it[1], lab(st[3])), label=lab(st[3]), cond=cond)
    elif fn == 'OGL_string' and S(st[1]):
        row('text', xm[0], xm[1], xm[2] if num(xm[2]) else 9999.0, it[0] if isinstance(it[0], int) else 0, label=S(st[1]), cond=cond)
    elif fn == 'BTN_tab':
        row('tab', it[0], it[1], it[3], key=it[2][1:] if isinstance(it[2], str) else 'tab', label=S(st[4]) or S(st[3]), cond=cond)
    elif fn.startswith('BTN_enterstring'):
        row('enter', xm[0], xm[1], xm[2] if num(xm[2]) else 200.0, key=key(it[0], lab(st[2])), label=lab(st[2]), cond=cond)
    elif fn == 'BTN_display_number':
        row('display', xm[0], xm[1], xm[2], it[0], it[1], label=lab(st[2]), cond=cond)
INST = {0: [0x114bc88, 0x114bd80, 0x114bee0, 0x11643b8], 1: [0x114bf48, 0x114bfc0, 0x114c040, 0x114c098, 0x11643b8],
        2: [0x114c130, 0x114c158, 0x114c1b0, 0x114c218, 0x11643b8], 3: [0x114c2f8, 0x114c348, 0x114c388, 0x114c3d8, 0x11643b8],
        4: [0x114c468, 0x114c4f0, 0x11643b8]}
if name == 'inst_wind':
    # The tab's paragraphs are concatenated at run time (not seen by btnemu); the strings are the
    # binary's own, in the order they sit in __cstring, one block per tab.
    B = open(os.environ['XP_INSTALL'] + '/X-Plane.app/Contents/MacOS/X-Plane', 'rb').read()
    cs = lambda a: B[a - 0x100000000 + 0x100000000 - 0x100000000 + 0x100000000 - 0x100000000: B.index(b'\0', a - 0x100000000)].decode('latin-1')
    for t, addrs in INST.items():
        text = '¶'.join(B[a - 0x100000000:B.index(b'\0', a - 0x100000000)].decode('latin-1') for a in addrs)
        row('text', 30.0, float(H) - 85.0, 540.0, 56, label=text, cond=f'inst_wind()::insW_tab={t}')
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), '../../..') + f'/crates/xp-core/data/windows/{wid}.tsv'
open(out, 'w').write(f'# kind\tx\ty\ta\tb\tc\td\tkey\tlabel\titems\tcond — start_win {wid}, {name} ({W}x{H}),\n# generated by re/gen_window.py from the BTN_* calls re/btnemu.py records. x, y: GL window coordinates.\n# size {W} {H}\n' + '\n'.join(rows) + '\n')
print(wid, name, len(rows), 'rows')
