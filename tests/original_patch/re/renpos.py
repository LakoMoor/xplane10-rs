import os
# Position (x, y GL bottom-up, window coords) of rendering setting #u in reno_wind, transcribed
# from the decompiled switch (out/reno/0x10076b7a0.c lines ~2328-2430).
def pos(u):
    x, y = 0x82, 0x2b2
    if u == 0xe: return x, y
    x, y = 0x32, 0x226
    if u == 0: return x, y
    y = 0x208
    if u == 1: return x, y
    u5 = 0x1ea; i16 = None
    if u == 2: i16 = 0x32; L = 'L537'
    else:
        u5 = 0x1cc
        if u == 0xf: i16 = 0x32; L = 'L537'
        else:
            i16 = 10; u5 = 10; x = 0xfa; y = 0x226
            if u == 3: return x, y
            if u == 4: u5 = 0x208
            elif u != 5: L = 'L537'
            else: u5 = 0x1ea
            if u in (4, 5): i16 = 0xfa; L = 'L950'
    while True:
        if L == 'L537':
            x, y = 0xfa, 0x1cc
            if u == 6: return x, y
            if u == 7: u5 = 0x226
            elif u != 8: L = 'L950'; continue
            else: u5 = 0x208
            i16 = 500; L = 'L565'; continue
        if L == 'L950':
            x, y = 500, 0x1ea
            if u == 9: return x, y
            y = 0x1cc
            if u == 10: return x, y
            x, y = 0x271, 0x208
            if u == 0x19: return x, y
            L = 'L565'; continue
        if L == 'L565':
            x, y = 800, 0x226
            if u == 0xb: return x, y
            if u == 0xc: u5 = 0x208; L = 'L58e'; continue
            if u == 0x13: u5 = 0x1ea; L = 'L58e'; continue
            x, y = 800, 0x1cc
            if u == 0xd: return x, y
            if u != 0x16:
                if u == 0x17: u5 = 0x168; L = 'L9d5'; continue
                L = 'L593'; continue
            u5 = 0x186; L = 'L9d5'; continue
        if L == 'L58e': i16 = 800; L = 'L593'; continue
        if L == 'L593':
            x, y = 0x32, 0x14a
            if u == 0x10: return x, y
            y = 0x186
            if u == 0x11: x = 0xfa; return x, y
            y = 0x168
            if u == 0x12: x = 0xfa; return x, y
            L = 'L9da'; continue
        if L == 'L9d5': i16 = 0x32; L = 'L9da'; continue
        if L == 'L9da':
            x, y = 0xfa, 0x14a
            if u == 0x14: return x, y
            x, y = 0x1db, 0x186
            if u == 0x15: return x, y
            x = i16
            if u == 0x18: x = 0x1db
            y = 0x14a
            if u != 0x18: y = u5
            return x, y

def settings(path):
    out = []
    for l in open(path, encoding='latin-1'):
        p = l.split(None, 1)
        if not p: continue
        if p[0] == 'SETTING':
            t = p[1].split(); out.append({'kind': t[0], 'ui': t[1], 'name': t[2], 'max': int(t[3])})
        elif p[0] == 'SETTING_NAME': out[-1]['label'] = p[1].rstrip('\n')
        elif p[0] == 'SETTING_MENU': out[-1]['menu'] = p[1].rstrip('\n')
        elif p[0] == 'SETTING_DEFAULT': out[-1].setdefault('default', p[1].split())
    return out

if __name__ == '__main__':
    S = settings(os.environ['XP_INSTALL'] + '/Resources/settings.txt')
    for i, s in enumerate(S):
        x, y = pos(i)
        print(i, s['ui'], s['name'], (x, y, 768 - y), s.get('label'), s.get('menu', ''), s.get('default'))
