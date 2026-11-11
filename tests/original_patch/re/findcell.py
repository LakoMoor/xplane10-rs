import os
# findcell.py <frame.png> x0 y0 x1 y1 [n] — best places in interface.png for a frame region,
# compositing the atlas over the region's own corner colour (rms over opaque-ish texels).
import sys, numpy as np
from PIL import Image
A = np.asarray(Image.open(os.environ['XP_INSTALL'] + '/Resources/bitmaps/interface.png').convert('RGBA')).astype(np.float32)
F = np.asarray(Image.open(sys.argv[1]).convert('RGB')).astype(np.float32)
x0, y0, x1, y1 = map(int, sys.argv[2:6]); n = int(sys.argv[6]) if len(sys.argv) > 6 else 5
P = F[y0:y1, x0:x1]; h, w = P.shape[:2]
bg = P[0, 0]
rgb, a = A[..., :3], A[..., 3:4] / 255.0
C = rgb * a + bg * (1 - a)               # atlas over the background
best = []
for ay in range(0, 512 - h):
    win = np.lib.stride_tricks.sliding_window_view(C[ay:ay + h], (h, w, 3))[0, :, 0]  # (512-w+1, h, w, 3)
    e = ((win - P) ** 2).mean(axis=(1, 2, 3))
    for ax in np.argsort(e)[:3]:
        best.append((float(e[ax]) ** .5, int(ax), ay))
best.sort()
for r in best[:n]: print('rms %.2f at atlas (%d, %d) size %dx%d' % (r[0], r[1], r[2], w, h))
