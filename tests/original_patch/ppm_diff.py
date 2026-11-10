"""ppm_diff.py <port.ppm> <oracle.png> <out.png> — save the port frame as PNG and print the
mean absolute difference per channel against the oracle frame (8-bit RGB PNG, same size).
Standard library only."""
import struct, sys, zlib

def read_ppm(p):
    d = open(p, 'rb').read(); a = d.split(b'\n', 3); w, h = map(int, a[1].split()); return w, h, a[3]

def read_png(p):
    d = open(p, 'rb').read(); i = 8; idat = b''; w = h = 0
    while i < len(d):
        n, t = struct.unpack('>I4s', d[i:i+8]); body = d[i+8:i+8+n]; i += 12 + n
        if t == b'IHDR':
            w, h, depth, ct, _, _, il = struct.unpack('>IIBBBBB', body)
            assert (depth, ct, il) == (8, 2, 0), "want 8-bit RGB, non-interlaced"
        elif t == b'IDAT': idat += body
    raw = zlib.decompress(idat); bpp = 3; stride = w * bpp; out = bytearray(); prev = bytearray(stride); o = 0
    for _ in range(h):
        f = raw[o]; line = bytearray(raw[o+1:o+1+stride]); o += 1 + stride
        for x in range(stride):
            a = line[x-bpp] if x >= bpp else 0; b = prev[x]; c = prev[x-bpp] if x >= bpp else 0
            if f == 1: line[x] = (line[x] + a) & 255
            elif f == 2: line[x] = (line[x] + b) & 255
            elif f == 3: line[x] = (line[x] + ((a + b) >> 1)) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += line; prev = line
    return w, h, bytes(out)

def write_png(p, w, h, rgb):
    raw = b''.join(b'\x00' + rgb[y*w*3:(y+1)*w*3] for y in range(h))
    ch = lambda t, b: struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b))
    open(p, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw, 9)) + ch(b'IEND', b''))

w, h, port = read_ppm(sys.argv[1]); write_png(sys.argv[3], w, h, port)
ow, oh, orc = read_png(sys.argv[2])
if (ow, oh) != (w, h):
    print(f"size differs: port {w}x{h}, oracle {ow}x{oh}"); sys.exit(0)
n = w * h
mad = [sum(abs(port[i] - orc[i]) for i in range(c, n * 3, 3)) / n for c in range(3)]
close = sum(1 for i in range(0, n * 3, 3) if max(abs(port[i+k] - orc[i+k]) for k in range(3)) <= 16) * 100 / n
print(f"vs oracle: mean |diff| R {mad[0]:.1f} G {mad[1]:.1f} B {mad[2]:.1f} of 255; {close:.1f} % of pixels within 16")
print(f"saved {sys.argv[3]}")
