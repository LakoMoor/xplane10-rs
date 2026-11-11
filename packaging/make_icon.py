"""make_icon.py <out.png> [size] — the xplane10-rs application icon, drawn from scratch (no X-Plane
or Laminar Research artwork): a high-wing light aircraft seen from above, climbing toward the upper
right, on a rounded square in the blue the original paints behind its dialogs (palette 0x2b).
Standard library only; 4x supersampled scanline fill."""
import sys, math, struct, zlib
N = int(sys.argv[2]) if len(sys.argv) > 2 else 1024
SS = 4; S = N * SS
def rot(pts, a, cx=0.5, cy=0.5):
    c, s = math.cos(a), math.sin(a)
    return [(cx + (x - cx) * c - (y - cy) * s, cy + (x - cx) * s + (y - cy) * c) for x, y in pts]
# aircraft in unit square, nose up (y grows downward)
fus = [(0.485,0.16),(0.515,0.16),(0.535,0.22),(0.53,0.62),(0.515,0.78),(0.485,0.78),(0.47,0.62),(0.465,0.22)]
wing = [(0.16,0.33),(0.84,0.33),(0.84,0.405),(0.16,0.405)]
tail = [(0.36,0.72),(0.64,0.72),(0.64,0.775),(0.36,0.775)]
fin  = [(0.492,0.66),(0.508,0.66),(0.508,0.80),(0.492,0.80)]
prop = [(0.43,0.145),(0.57,0.145),(0.57,0.158),(0.43,0.158)]
stripe=[(0.47,0.45),(0.53,0.45),(0.53,0.47),(0.47,0.47)]
A = math.radians(35)
shapes = [(rot(p, A), (245,246,248)) for p in (wing, tail, fus, fin)] + [(rot(prop, A),(40,44,52)), (rot(stripe, A),(232,120,40))]
def covers(poly, y):
    xs = []
    n = len(poly)
    for i in range(n):
        (x1,y1),(x2,y2) = poly[i], poly[(i+1)%n]
        if (y1 <= y < y2) or (y2 <= y < y1):
            xs.append(x1 + (y - y1) * (x2 - x1) / (y2 - y1))
    xs.sort(); return [(xs[i], xs[i+1]) for i in range(0, len(xs) - 1, 2)]
R = 0.2237  # macOS-style corner radius
def inside_rr(u, v):
    m = 0.04; l, t, r, b = m, m, 1 - m, 1 - m
    cx = min(max(u, l + R), r - R); cy = min(max(v, t + R), b - R)
    return l <= u <= r and t <= v <= b and (u - cx) ** 2 + (v - cy) ** 2 <= R * R
acc = [[0.0]*4 for _ in range(N*N)]
for sy in range(S):
    v = (sy + 0.5) / S; row = sy // SS
    spans = [(covers(p, v), c) for p, c in shapes]
    for sx in range(S):
        u = (sx + 0.5) / S
        if not inside_rr(u, v): continue
        # background: vertical gradient from the dialog blue to a lighter sky blue
        k = v
        col = (13 + (60-13)*(1-k), 69 + (130-69)*(1-k), 101 + (175-101)*(1-k))
        for sp, c in spans:
            if any(a <= u < b for a, b in sp): col = c
        # soft horizon line
        a = acc[row * N + sx // SS]
        a[0] += col[0]; a[1] += col[1]; a[2] += col[2]; a[3] += 1
out = bytearray()
for y in range(N):
    out.append(0)
    for x in range(N):
        a = acc[y*N + x]; n = SS*SS
        al = a[3] / n
        if a[3]: out += bytes((round(a[0]/a[3]), round(a[1]/a[3]), round(a[2]/a[3]), round(al*255)))
        else: out += b'\0\0\0\0'
ch = lambda t, b: struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b))
open(sys.argv[1], 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', N, N, 8, 6, 0, 0, 0)) + ch(b'IDAT', zlib.compress(bytes(out), 9)) + ch(b'IEND', b''))
