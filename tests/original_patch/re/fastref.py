# fast rip-relative reference finder: for each target, list code addresses whose disp32 resolves to it
import sys, struct, bisect, re
b = open(sys.argv[1], 'rb').read()
toff, tsize, taddr = 6080, 0x1085302, 0x1000017c0
text = b[toff:toff+tsize]
syms = []
for l in open('syms.txt'):
    p = l.rstrip('\n').split(' ', 2)
    if len(p) == 3 and len(p[0]) == 16: syms.append((int(p[0], 16), p[2]))
syms.sort(); addrs = [s[0] for s in syms]
def sym(a):
    i = bisect.bisect_right(addrs, a) - 1
    return f"{syms[i][1][:110]}+{a-addrs[i]:#x}"
def refs(target):
    out = []
    # the disp high 2 bytes vary slowly: for window of 64K positions the high bytes take <=2 values; search by regex on high 16 bits
    for k in (0, 1, 4):
        base = target - taddr - 4 - k
        seen = set()
        for i0 in range(0, len(text), 0x8000):
            for d in (base - i0, base - i0 - 0x7fff):
                hi = struct.pack('<i', d)[2:]
                if (i0, hi) in seen: continue
                seen.add((i0, hi))
                j = text.find(hi, i0 + 2, i0 + 0x8000 + 4)
                while j != -1:
                    i = j - 2
                    if struct.unpack_from('<i', text, i)[0] == base - i:
                        out.append(taddr + i)
                    j = text.find(hi, j + 1, i0 + 0x8000 + 4)
    return sorted(set(out))
if __name__ == '__main__':
    for t in sys.argv[2:]:
        print("==", t)
        for a in refs(int(t, 16)): print("  ", hex(a), sym(a))
