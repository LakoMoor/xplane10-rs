import os
# gen_window.py <asm name> <start_win id> <W> <H> : widget table for one window from btnemu.py
import json, subprocess, sys
name, wid = sys.argv[1], int(sys.argv[2])
# Window sizes set by wins_class::start_win (0x10040fe60); the others fill the screen.
FIXED = {1: (651, 753), 5: (572, 268), 9: (620, 438), 10: (1020, 766), 11: (700, 500), 15: (530, 650),
         17: (1020, 750), 18: (831, 636), 19: (600, 400), 20: (550, 425), 21: (550, 300), 22: (1024, 768)}
def run(W, H, *extra):
    return json.loads(subprocess.run(['python3', 'btnemu.py', f'out/asm/{name}.asm', str(W), str(H), *extra], capture_output=True, text=True).stdout)
def build(W, H):
    global rows
    rows = []
    calls = run(W, H)
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
            for c in run(W, H, f'--tab={var}={v}'):
                seen.setdefault((int(c['pc'], 16), c.get('it', '')), (c, set()))[1].add(v)
        calls = [seen[k][0] for k in sorted(seen)]
        for pc, (c, vs) in seen.items():
            if len(vs) < len(values):
                TABCOND[pc] = var + '=' + '/'.join(str(v) for v in sorted(vs))
    rows = []
    def f(v):
        if isinstance(v, float): return '%g' % v
        if v is None: return ''
        return str(v)
    def row(kind, x, y, a='', b='', c='', d='', key='', label='', items='', cond=''):
        rows.append([kind, x, y, a, b, c, d, key, str(label).replace('\t', ' ').replace('\n', ' '), items, cond])
    def S(v): return v if isinstance(v, str) and v != '?' and not v.startswith('stk') else ''
    def lab(v):
        t = S(v)
        return '' if t.startswith('{') else t.split(' { ')[0]
    def key(v, label):
        if isinstance(v, str) and v.startswith('&'): return v[1:]
        if isinstance(v, list) and v and v[0] == 'thisp': return 'this+0x%x' % v[1]
        if isinstance(v, list) and v and v[0] == 'mem': return 'm:' + str(v[1])
        return 'lbl:' + (label or 'anon%d' % len(rows))
    tabvars = {c['int'][2][1:] for c in calls if c['fn'].startswith('BTN_tab') and isinstance(c['int'][2], str)}
    for c in calls:
        fn, xm, it, st = c['fn'].split('(')[0], c['xmm'], c['int'], c['str']
        cond = TABCOND.get((int(c['pc'], 16), c.get('it', '')), '')
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
            row('text', xm[0], xm[1], xm[2] if num(xm[2]) else 9999.0, it[0] if isinstance(it[0], int) else 0, it[3] if isinstance(it[3], int) and it[3] in (0, 1, 2) else 0, label=S(st[1]), cond=cond)
        elif fn == 'BTN_tab':
            row('tab', it[0], it[1], it[3], key=it[2][1:] if isinstance(it[2], str) else 'tab', label=S(st[4]) or S(st[3]), cond=cond)
        elif fn.startswith('BTN_enterstring'):
            row('enter', xm[0], xm[1], xm[2] if num(xm[2]) else 200.0, key=key(it[0], lab(st[2])), label=lab(st[2]), cond=cond)
        elif fn == 'OGL_number' and num(xm[2]) is not None and isinstance(it[2], int):
            row('text', xm[0], xm[1], 9999.0, it[0] if isinstance(it[0], int) else 52, label='%.*f' % (it[2], xm[2]), cond=cond)
        elif fn == 'BTN_frame1' and it[0] == 9 and num(xm[2]) is not None:
            row('field', xm[0], xm[1], xm[2], cond=cond)
        elif fn == 'BTN_display_number':
            row('display', xm[0], xm[1], xm[2], it[0], it[1], label=lab(st[2]), cond=cond)

    extras(W, H, row)
    # Same widget reached on two code paths: keep one. Positions the walk could not resolve: drop.
    out, seen_rows = [], set()
    for r in rows:
        k = tuple(str(v) for v in r)
        if k in seen_rows or not all(isinstance(v, (int, float)) for v in r[1:3]): continue
        if not (-50 <= r[1] <= W + 50 and -50 <= r[2] <= H + 50): continue   # walk lost track
        if r[0] == 'text':
            # Texts at one spot are alternatives picked at run time (status lines): keep one.
            spot = ('text@', r[1], r[2], r[10])
            if spot in seen_rows: continue
            seen_rows.add(spot)
        seen_rows.add(k); out.append(r)
    return out
INST = {0: [0x114bc88, 0x114bd80, 0x114bee0, 0x11643b8], 1: [0x114bf48, 0x114bfc0, 0x114c040, 0x114c098, 0x11643b8],
        2: [0x114c130, 0x114c158, 0x114c1b0, 0x114c218, 0x11643b8], 3: [0x114c2f8, 0x114c348, 0x114c388, 0x114c3d8, 0x11643b8],
        4: [0x114c468, 0x114c4f0, 0x11643b8]}
def extras(W, H, row):
  if name == 'inst_wind':
    # The tab's paragraphs are concatenated at run time (not seen by btnemu); the strings are the
    # binary's own, in the order they sit in __cstring, one block per tab.
    B = open(os.environ['XP_INSTALL'] + '/X-Plane.app/Contents/MacOS/X-Plane', 'rb').read()
    cs = lambda a: B[a - 0x100000000 + 0x100000000 - 0x100000000 + 0x100000000 - 0x100000000: B.index(b'\0', a - 0x100000000)].decode('latin-1')
    for t, addrs in INST.items():
        text = '¶'.join(B[a - 0x100000000:B.index(b'\0', a - 0x100000000)].decode('latin-1') for a in addrs)
        row('text', 30.0, float(H) - 85.0, 540.0, 56, label=text, cond=f'inst_wind()::insW_tab={t}')
  if name == 'dout_wind':
    # Data Set rows: dout_class::get_line_object(io, i) names each line (jump table at
    # 0x100458918; 119 and 133 are built outside it). Drawn 66 right of the row's first box.
    names = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'dout_names.json')))
    names[119], names[133] = 'weapon status', 'cruise stats'
    starts = [40.0, 295.0, 550.0, 805.0]
    for r in list(rows):
        if r[0] == 'text' and str(r[8]).isdigit() and isinstance(r[1], (int, float)) and r[10].endswith('datW_tab=0'):
            col = min((c for c in starts if c > r[1]), default=None)
            if col is not None and int(r[8]) < len(names):
                row('text', col + 64.0, r[2], 9999.0, 52, label=names[int(r[8])], cond=r[10])
  if name == 'xtim_wind':
    # Month names: a loop over a string table, x = (i - 5.5) * 500 / 12 + 286 - 12 (xtim_wind).
    for i, m in enumerate('Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec'.split()):
        row('text', (i - 5.5) * 500.0 / 12.0 + 274.0, 68.0, 9999.0, 52, key='', label=m)
    # Clock read-outs, formatted at run time from the sim clock.
    row('text', 35.0, 154.0, 9999.0, 54, key='clock:local')
    row('text', 120.0, 154.0, 9999.0, 54, key='clock:zulu')
    row('text', 155.0, 34.0, 9999.0, 54, 2, key='clock:date')

def f(v):
    if isinstance(v, float): return '%g' % v
    if v is None: return ''
    return str(v)
def numeric(v): return isinstance(v, (int, float)) and not isinstance(v, bool)
if wid in FIXED:
    W, H = FIXED[wid]
    lines = ['\t'.join(f(v) for v in r) for r in build(W, H)]
    size = f'{W} {H}'
else:
    # Full-screen window: walk it at three screen sizes; a number that moves with the screen is
    # written base~kw,kh = base + kw*(W-1024) + kh*(H-768).
    W, H = 1024, 768
    a, b, c = build(1024, 768), build(1124, 768), build(1024, 868)
    # Pair rows by what they are (kind, key, label, items, cond), in order; a row with no partner
    # keeps fixed numbers.
    def ident(r): return (r[0], r[7], r[8], r[9], r[10])
    def pairing(other):
        pool = {}
        for r in other: pool.setdefault(ident(r), []).append(r)
        return [pool[ident(r)].pop(0) if pool.get(ident(r)) else r for r in a]
    lines = []
    for ra, rb, rc in zip(a, pairing(b), pairing(c)):
        cells = []
        for i, (va, vb, vc) in enumerate(zip(ra, rb, rc)):
            if i in (1, 2, 3, 4, 5, 6) and numeric(va) and numeric(vb) and numeric(vc) and (vb != va or vc != va):
                cells.append('%s~%g,%g' % (f(float(va)), (vb - va) / 100.0, (vc - va) / 100.0))
            else:
                cells.append(f(va))
        lines.append('\t'.join(cells))
    size = 'screen'
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), '../../..') + f'/crates/xp-core/data/windows/{wid}.tsv'
open(out, 'w').write(f'# kind\tx\ty\ta\tb\tc\td\tkey\tlabel\titems\tcond — start_win {wid}, {name},\n# generated by re/gen_window.py from the BTN_* calls re/btnemu.py records. x, y: GL window coordinates;\n# base~kw,kh = base + kw*(screen W - 1024) + kh*(screen H - 768).\n# size {size}\n' + '\n'.join(lines) + '\n')
print(wid, name, len(lines), 'rows', size)
