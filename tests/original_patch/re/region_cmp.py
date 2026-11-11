"""region_cmp.py <port.ppm|png> <oracle.png> x0 y0 x1 y1 [zoom out.png] — compare a screen region pixel by pixel."""
import sys; import os; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from png import read_png, write_png
def load(p):
    if p.endswith('.ppm'):
        d = open(p, 'rb').read(); a = d.split(b'\n', 3); w, h = map(int, a[1].split()); return w, h, 3, a[3]
    return read_png(p)
w, h, b, P = load(sys.argv[1]); W, H, B, O = read_png(sys.argv[2])
x0, y0, x1, y1 = map(int, sys.argv[3:7])
n = eq = c4 = 0; tot = 0; worst = (0, 0, 0)
for y in range(y0, y1):
    for x in range(x0, x1):
        p = P[(y*w+x)*b:(y*w+x)*b+3]; o = O[(y*W+x)*B:(y*W+x)*B+3]
        d = max(abs(p[k]-o[k]) for k in range(3)); n += 1; eq += d == 0; c4 += d <= 4; tot += d
        if d > worst[0]: worst = (d, x, y)
print(f"region {x0},{y0}-{x1},{y1}: identical {eq*100/n:.1f} %, within 4 {c4*100/n:.1f} %, mean {tot/n:.2f}, worst {worst}")
if len(sys.argv) > 8:
    z = int(sys.argv[7]); out = bytearray(); ww = x1 - x0
    for y in range((y1-y0)*z):
        for src in (P, O):
            bb, wd = (b, w) if src is P else (B, W)
            for x in range(ww*z):
                i = ((y0+y//z)*wd + x0 + x//z)*bb; out += src[i:i+3]
    write_png(sys.argv[8], ww*z*2, (y1-y0)*z, bytes(out))
