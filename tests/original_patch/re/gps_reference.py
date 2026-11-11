import os
import sys, os, math, struct
root = sys.argv[1]
f32 = lambda x: struct.unpack('<f', struct.pack('<f', x))[0]
D2R = 0.01745329424738884
def rwy_len(lat1, lon1, lat2, lon2):
    clat = (lat1 + lat2) * 0.5
    dlat = f32(lat2 - lat1)
    c = f32(math.cos(f32(D2R * clat)))
    dx = f32((lon2 - lon1) * c)
    s = f32(math.sqrt(f32(f32(dx*dx) + f32(dlat*dlat))))
    return f32(s * 111120.0)
files = [
 "Custom Scenery/KLAL - Lakeland (airshow version)/Earth nav data/apt.dat",
 "Custom Scenery/KOSH - Wittman Regional (airshow version)/Earth nav data/apt.dat",
 "Custom Scenery/KSEA Demo Area/Earth nav data/apt.dat",
 "Custom Scenery/LOWI Demo Area/Earth nav data/apt.dat",
 "Custom Scenery/Global Airports/Earth nav data/apt.dat",
 "Resources/default scenery/default apt dat/Earth nav data/apt.dat",
]
apts = {}
for rel in files:
    cur = None
    with open(os.path.join(root, rel), 'rb') as f:
        f.readline(); f.readline()
        for line in f:
            p = line.split()
            if not p: continue
            k = p[0]
            if k in (b'1', b'16', b'17'):
                i = p[4].decode('latin1')
                if i in apts: cur = None; continue
                cur = dict(kind=int(k), land=[], water=[], heli=0); apts[i] = cur
            elif cur is None: continue
            elif k == b'100':
                cur['land'].append((rwy_len(float(p[9]), float(p[10]), float(p[18]), float(p[19])), int(p[2])))
            elif k == b'101':
                cur['water'].append(rwy_len(float(p[4]), float(p[5]), float(p[7]), float(p[8])))
            elif k == b'102': cur['heli'] += 1
gns = set(l.split(',')[1] for l in open(os.path.join(root, "Resources/GNS430/navdata/Airports.txt"), encoding='latin1') if l.startswith('A,'))
new = lambda i: i not in gns and 'K'+i not in gns
T = 19186
ths = {"f32(213.36)": f32(213.36), "f32(700)*f32(.3048)": f32(f32(700.0)*f32(0.3048)), "700/3.28084": f32(700/3.28084), "700/3.28": f32(700/3.28), "213.0": 213.0}
for n, th in ths.items():
    for kn, kf in (("kind1", lambda a: a['kind'] == 1), ("any", lambda a: True)):
        for wn, lens in (("land", lambda a: [r[0] for r in a['land']]), ("land+water", lambda a: [r[0] for r in a['land']] + a['water'])):
            c = sum(1 for i, a in apts.items() if new(i) and kf(a) and any(not (th > L) for L in lens(a)))
            print(f"{c:6d} {c-T:+4d} th={n}={th!r} {kn} {wn}")
for i, a in apts.items():
    if new(i) and a['land']:
        m = max(r[0] for r in a['land'])
        if 213.2 <= m <= 213.5: print(i, a['kind'], repr(m))
