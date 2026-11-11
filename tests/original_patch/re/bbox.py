import os
# bbox.py <png> x0 y0 x1 y1 [thr] : bounding box and rows of "ink" (darker than thr, or brighter
# if thr<0) inside a region, plus row ranges of separate text lines.
import sys, numpy as np
from PIL import Image
F = np.asarray(Image.open(sys.argv[1]).convert('L')).astype(int)
x0, y0, x1, y1 = map(int, sys.argv[2:6]); t = int(sys.argv[6]) if len(sys.argv) > 6 else 120
R = F[y0:y1, x0:x1]; M = (R < t) if t > 0 else (R > -t)
ys, xs = np.nonzero(M)
if len(xs) == 0: print('none'); sys.exit()
print('bbox x %d..%d y %d..%d' % (x0 + xs.min(), x0 + xs.max(), y0 + ys.min(), y0 + ys.max()))
rows = sorted(set(ys)); runs = []; s = p = rows[0]
for r in rows[1:]:
    if r != p + 1: runs.append((s, p)); s = r
    p = r
runs.append((s, p))
for a, b in runs:
    sub = M[a:b + 1]; xx = np.nonzero(sub.any(0))[0]
    print('  line y %d..%d x %d..%d' % (y0 + a, y0 + b, x0 + xx.min(), x0 + xx.max()))
