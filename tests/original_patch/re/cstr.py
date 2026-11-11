import os
"""cstr.py <file.c> — annotate Ghidra C with the C strings and symbol names its constants point at."""
import sys, re, bisect, os
HERE = os.path.dirname(os.path.abspath(__file__))
b = open(os.environ["XP_INSTALL"] + "/X-Plane.app/Contents/MacOS/X-Plane", 'rb').read()
syms = []
for l in open(os.path.join(HERE, 'syms.txt')):
    p = l.rstrip('\n').split(' ', 2)
    if len(p) == 3 and len(p[0]) == 16: syms.append((int(p[0], 16), p[2]))
syms.sort(); addrs = [s[0] for s in syms]
def cs(a):
    o = a - 0x100000000
    if not (0x1087000 <= o < 0x14a8000): return None
    e = b.find(b'\0', o); s = b[o:e]
    if 1 < len(s) < 160 and all(32 <= c < 127 or c in (9, 10) for c in s): return repr(s.decode())
def sym(a):
    i = bisect.bisect_right(addrs, a) - 1
    if i < 0 or a - addrs[i] > 0x400: return None
    return syms[i][1][:60] + (f"+{a-addrs[i]:#x}" if a != addrs[i] else "")
for l in open(sys.argv[1]):
    l = l.rstrip('\n'); notes = []
    for m in re.finditer(r'(?:DAT_|0x|func_0x0*|s_\w*?_)(10[0-9a-f]{7})\b', l):
        a = int(m.group(1), 16); n = cs(a) or (sym(a) if a < 0x1014a8000 or a >= 0x102900000 else None)
        if n and n not in notes: notes.append(n)
    print(l + ("   // " + " | ".join(notes) if notes else ""))
