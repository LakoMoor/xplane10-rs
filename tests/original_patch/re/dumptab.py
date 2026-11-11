import os
"""dumptab.py <binary> <addr> <count> — print a table of C-string pointers from a Mach-O data section."""
import struct, re, subprocess, sys
B = sys.argv[1]; b = open(B, 'rb').read()
out = subprocess.run(['otool', '-l', B], capture_output=True, text=True).stdout
secs = [(int(m.group(1), 16), int(m.group(2), 16), int(m.group(3)))
        for m in re.finditer(r'sectname \S+\n\s+segname \S+\n\s+addr (0x[0-9a-f]+)\n\s+size (0x[0-9a-f]+)\n\s+offset (\d+)', out)]
def off(a):
    for ad, sz, o in secs:
        if ad <= a < ad + sz and o: return o + (a - ad)
a = int(sys.argv[2], 16)
for i in range(int(sys.argv[3])):
    o = off(a + 8 * i)
    p = struct.unpack_from('<Q', b, o)[0]
    po = off(p) if p else None
    s = b[po:b.find(b'\0', po)].decode('latin1') if po else ''
    print(f"{i}\t{s}")
