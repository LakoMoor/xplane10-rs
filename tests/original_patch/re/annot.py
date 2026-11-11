import os
import sys, re, bisect
B = sys.argv[1]
syms = []
for l in open('syms.txt'):
    p = l.rstrip('\n').split(' ', 2)
    if len(p) == 3 and len(p[0]) == 16:
        syms.append((int(p[0], 16), p[2]))
syms.sort()
addrs = [s[0] for s in syms]
data = open(B, 'rb').read()
def sym(a):
    i = bisect.bisect_right(addrs, a) - 1
    if i < 0: return None
    off = a - addrs[i]
    return syms[i][1] + (f"+{off:#x}" if off else "")
def cstr(a):
    off = a - 0x100000000
    if 0x1144c00 <= off < 0x1144c00 + 0xedffb + 0x400000:
        e = data.find(b'\0', off)
        s = data[off:e]
        if 0 < len(s) < 200 and all(32 <= c < 127 or c in (9, 10) for c in s):
            return repr(s.decode())
    return None
for l in sys.stdin:
    l = l.rstrip('\n')
    m = re.search(r'(call|jmp|j[a-z]+)\s+0x([0-9a-f]+)', l)
    note = None
    if m and m.group(1) == 'call':
        note = sym(int(m.group(2), 16))
    m2 = re.search(r'# 0x([0-9a-f]+)', l)
    if m2:
        a = int(m2.group(1), 16)
        note = cstr(a) or sym(a)
    print(l + ("    ; " + note if note else ""))
