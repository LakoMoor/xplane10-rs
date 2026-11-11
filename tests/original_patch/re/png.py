import os
"""Minimal PNG read/write (8-bit RGB/RGBA, non-interlaced), standard library only."""
import struct, zlib
def read_png(p):
    d = open(p, 'rb').read(); i = 8; idat = b''
    while i < len(d):
        n, t = struct.unpack('>I4s', d[i:i+8]); body = d[i+8:i+8+n]; i += 12 + n
        if t == b'IHDR': w, h, depth, ct, _, _, il = struct.unpack('>IIBBBBB', body); assert depth == 8 and il == 0 and ct in (2, 6)
        elif t == b'IDAT': idat += body
    bpp = 3 if ct == 2 else 4; raw = zlib.decompress(idat); stride = w * bpp; out = bytearray(); prev = bytearray(stride); o = 0
    for _ in range(h):
        f = raw[o]; line = bytearray(raw[o+1:o+1+stride]); o += 1 + stride
        if f == 1:
            for x in range(bpp, stride): line[x] = (line[x] + line[x-bpp]) & 255
        elif f == 2:
            for x in range(stride): line[x] = (line[x] + prev[x]) & 255
        elif f == 3:
            for x in range(stride): line[x] = (line[x] + (((line[x-bpp] if x >= bpp else 0) + prev[x]) >> 1)) & 255
        elif f == 4:
            for x in range(stride):
                a = line[x-bpp] if x >= bpp else 0; b = prev[x]; c = prev[x-bpp] if x >= bpp else 0
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += line; prev = line
    return w, h, bpp, bytes(out)
def write_png(p, w, h, rgb):
    raw = b''.join(b'\x00' + rgb[y*w*3:(y+1)*w*3] for y in range(h))
    ch = lambda t, b: struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b))
    open(p, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw, 9)) + ch(b'IEND', b''))
def crop_zoom(src, dst, x0, y0, w, h, z):
    W, H, b, P = read_png(src); out = bytearray()
    for y in range(h * z):
        for x in range(w * z):
            i = ((y0 + y // z) * W + x0 + x // z) * b; out += P[i:i+3]
    write_png(dst, w * z, h * z, bytes(out))
if __name__ == '__main__':
    import sys; a = sys.argv; crop_zoom(a[1], a[2], *map(int, a[3:8]))
