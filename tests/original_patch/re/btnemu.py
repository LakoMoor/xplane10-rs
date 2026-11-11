import os
# btnemu.py <asm file> [W H] — walk a window function's disassembly (objdump AT&T syntax) in
# address order and print every BTN_* / OGL_* call with concrete arguments.
#
# Models: xmm0-15 (scalar floats), integer registers holding immediates, global addresses or
# stack-object addresses, stack slots, std::string objects built from literals
# (std::string(const char*) stub 0x101086fe4, UTL_translate_string 0x1000ddab0, string copy),
# the layout globals written by BTN_subwin through its float* outputs.
# Linear sweep: branches are not followed, every call site is reported once, in order.
import json, re, struct, sys

X = os.environ["XP_INSTALL"] + "/X-Plane.app/Contents/MacOS/X-Plane"
B = open(X, 'rb').read()
SYMS = {}
for l in open(os.environ.get('RE_WORK', '.') + '/syms.txt'):
    p = l.rstrip('\n').split(' ', 2)
    if len(p) == 3 and len(p[0]) == 16:
        SYMS[int(p[0], 16)] = p[2]

def rd(a, n):
    o = a - 0x100000000
    return B[o:o + n]

def f32(a): return struct.unpack('<f', rd(a, 4))[0]
def f64(a): return struct.unpack('<d', rd(a, 8))[0]
def i32(a): return struct.unpack('<i', rd(a, 4))[0]
def cstr(a):
    o = a - 0x100000000; e = B.index(b'\0', o)
    return B[o:e].decode('latin-1')

W, H = (float(sys.argv[2]), float(sys.argv[3])) if len(sys.argv) > 3 else (1024.0, 768.0)
WINS = 0x102f2b220
mem = {}          # global float memory (address -> value) written during the walk
mem[WINS + 0x14] = W; mem[WINS + 0x18] = H
x = {}            # xmm registers
r = {}            # integer registers: ('imm', v) | ('ptr', addr) | ('stk', off) | ('str', s)
stk = {}          # stack slots: off -> value (float or tuple)
strs = {}         # stack string objects: off -> text
out = []
conds = []        # (var, imm, op, start_pc, end_pc): code between start and end runs only when var op imm
last_cmp = None

def gname(a):
    n = SYMS.get(a)
    if n: return n
    return hex(a)

def memval(a):
    if a in mem: return mem[a]
    return ('g', gname(a))

REG64 = {'eax':'rax','ebx':'rbx','ecx':'rcx','edx':'rdx','esi':'rsi','edi':'rdi','r8d':'r8','r9d':'r9',
         'r10d':'r10','r11d':'r11','r12d':'r12','r13d':'r13','r14d':'r14','r15d':'r15','ebp':'rbp'}
def rn(s):
    s = s.lstrip('%'); return REG64.get(s, s)

def addr_of(op, comment):
    """memory operand -> ('abs', addr) | ('stk', off) | ('ind', reg, disp)"""
    m = re.match(r'(-?0x[0-9a-f]+|-?\d+)?\(%(\w+)\)$', op)
    if not m: return None
    disp = int(m.group(1), 0) if m.group(1) else 0
    reg = m.group(2)
    if reg == 'rip':
        cm = re.search(r'## (0x[0-9a-f]+)', comment)
        return ('abs', int(cm.group(1), 16))
    if reg == 'rbp': return ('stk', disp)
    v = r.get(rn(reg))
    if v and v[0] == 'ptr': return ('abs', v[1] + disp)
    if v and v[0] == 'stk': return ('stk', v[1] + disp)
    return ('ind', reg, disp)

def load_f(op, comment, dbl=False):
    if op.startswith('%xmm'): return x.get(op[1:])
    a = addr_of(op, comment)
    if not a: return None
    if a[0] == 'abs':
        if a[1] in mem: return mem[a[1]]
        if 0x100000000 <= a[1] < 0x1014a8000: return f64(a[1]) if dbl else f32(a[1])
        return ('g', gname(a[1]))
    if a[0] == 'stk': return stk.get(a[1])
    return None

def store(op, comment, v):
    a = addr_of(op, comment)
    if not a: return
    if a[0] == 'abs': mem[a[1]] = v
    elif a[0] == 'stk': stk[a[1]] = v

def arith(opn, a, b):
    if not isinstance(a, (int, float)) or not isinstance(b, (int, float)):
        return ('expr', opn, a, b)
    return {'add': a + b, 'sub': a - b, 'mul': a * b, 'div': a / b if b else float('nan'),
            'max': max(a, b), 'min': min(a, b)}[opn]

def strarg(reg):
    v = r.get(reg)
    if v and v[0] == 'stk': return strs.get(v[1], '?')
    if v and v[0] == 'ptr': return ('g', gname(v[1]))
    return '?'

def callargs():
    return {'edi': r.get('rdi'), 'esi': r.get('rsi'), 'edx': r.get('rdx'), 'ecx': r.get('rcx'),
            'r8': r.get('r8'), 'r9': r.get('r9'),
            'xmm': [x.get('xmm%d' % i) for i in range(8)]}

def fmt(v):
    if isinstance(v, float): return round(v, 4)
    if isinstance(v, tuple):
        if v[0] == 'imm': return v[1]
        if v[0] == 'ptr': return '&' + gname(v[1])
        if v[0] == 'stk': return strs.get(v[1], 'stk%d' % v[1])
        return list(v)
    return v

def exe(pc, ins, ops, comment, o, l):
    global last_cmp
    try:
        if ins in ('movss', 'movsd', 'movaps', 'movapd', 'movd'):
            src, dst = o
            if dst.startswith('%xmm'):
                v = load_f(src, comment, ins == 'movsd')
                if ins == 'movd' and src.startswith('%e'):
                    rv = r.get(rn(src)); v = struct.unpack('<f', struct.pack('<i', rv[1]))[0] if rv and rv[0]=='imm' else None
                x[dst[1:]] = v
            elif src.startswith('%xmm'):
                store(dst, comment, x.get(src[1:]))
        elif ins in ('xorps', 'xorpd', 'pxor') and o[0] == o[1]:
            x[o[1][1:]] = 0.0
        elif ins in ('addss', 'subss', 'mulss', 'divss', 'maxss', 'minss', 'addsd', 'subsd', 'mulsd', 'divsd'):
            src, dst = o
            v = load_f(src, comment, ins.endswith('sd'))
            x[dst[1:]] = arith(ins[:3], x.get(dst[1:]), v)
        elif ins in ('cvtsi2ssl', 'cvtsi2ss', 'cvtsi2sdl', 'cvtsi2sd'):
            src, dst = o
            if src.startswith('%'):
                v = r.get(rn(src)); x[dst[1:]] = float(v[1]) if v and v[0] == 'imm' else ('int', fmt(v))
            else:
                a = addr_of(src, comment)
                x[dst[1:]] = float(i32(a[1])) if a and a[0] == 'abs' and a[1] < 0x1014a8000 else ('g', gname(a[1])) if a and a[0]=='abs' else None
        elif ins in ('cvtss2sd', 'cvtsd2ss'):
            x[o[1][1:]] = x.get(o[0][1:]) if o[0].startswith('%xmm') else load_f(o[0], comment, ins == 'cvtsd2ss')
        elif ins in ('cvttss2si', 'cvttsd2si'):
            v = x.get(o[0][1:]) if o[0].startswith('%xmm') else load_f(o[0], comment)
            r[rn(o[1])] = ('imm', int(v)) if isinstance(v, float) else ('val', fmt(v))
        elif ins in ('movl', 'movq', 'movabsq'):
            src, dst = o
            if src.startswith('$'):
                v = ('imm', int(src[1:], 0))
            elif src.startswith('%'):
                v = r.get(rn(src))
            else:
                a = addr_of(src, comment)
                v = None
                if a and a[0] == 'stk': v = stk.get(a[1])
                elif a and a[0] == 'abs':
                    v = ('mem', gname(a[1]))
            if dst.startswith('%'): r[rn(dst)] = v
            else: store(dst, comment, v)
        elif ins == 'leaq':
            src, dst = o
            a = addr_of(src, comment)
            r[rn(dst)] = ('ptr', a[1]) if a and a[0] == 'abs' else ('stk', a[1]) if a and a[0] == 'stk' else None
        elif ins in ('cmpl', 'cmpb') and o[0].startswith('$'):
            imm = int(o[0][1:], 0); tgt = o[1]
            var = None
            if tgt.startswith('%'):
                v = r.get(rn(tgt))
                if v and v[0] == 'mem': var = v[1]
            else:
                a = addr_of(tgt, comment)
                if a and a[0] == 'abs': var = gname(a[1])
            last_cmp = (var, imm, pc) if var else None
        elif ins == 'testl' and o[0] == o[1]:
            v = r.get(rn(o[0]))
            last_cmp = (v[1], 0, pc) if v and v[0] == 'mem' else None
        elif ins in ('jne', 'je') and last_cmp:
            t = int(o[0].split()[0], 16)
            if t > pc and ins == 'jne':
                conds.append((last_cmp[0], last_cmp[1], '==' if ins == 'jne' else '!=', pc, t))
            last_cmp = None
        elif ins == 'xorl' and o[0] == o[1]:
            r[rn(o[1])] = ('imm', 0)
        elif ins.startswith('call'):
            cm = re.search(r'<([^>]+)>', l)
            name = cm.group(1) if cm else '?'
            tgt = int(o[0].split()[0], 16) if o and o[0].startswith('0x') else None
            if tgt == 0x101086fe4:   # std::string(const char*)
                d, s = r.get('rdi'), r.get('rsi')
                if d and d[0] == 'stk':
                    strs[d[1]] = cstr(s[1]) if s and s[0] == 'ptr' else '?'
                r['rax'] = d
            elif tgt == 0x1000ddab0 or 'ZNSsC1ERKSs' in name:   # translate / copy
                d, s = r.get('rdi'), r.get('rsi')
                if d and d[0] == 'stk':
                    strs[d[1]] = strs.get(s[1], '?') if s and s[0] == 'stk' else ('g', gname(s[1])) if s and s[0]=='ptr' else '?'
            else:
                fn = SYMS.get(tgt, name) if tgt else name
                if any(k in fn for k in ('BTN_', 'OGL_', 'set_view_type', 'REN_', 'MACIBM_set', 'is_int', 'textlist', 'dob_range', 'slider', 'qflt', 'wins_class')):
                    a = callargs()
                    active = [f'{c[0]}{c[2]}{c[1]}' for c in conds if c[3] < pc < c[4]]
                    out.append({'pc': hex(pc), 'fn': fn, 'cond': active,
                                'int': [fmt(a[k]) for k in ('edi', 'esi', 'edx', 'ecx', 'r8', 'r9')],
                                'str': [strarg(k) for k in ('rdi', 'rsi', 'rdx', 'rcx', 'r8', 'r9')],
                                'xmm': [fmt(v) for v in a['xmm']]})
                    if 'BTN_subwin' in fn:
                        xm = a['xmm']
                        def num(v, d): return v if isinstance(v, float) else d
                        bx = num(xm[1], -1.0); by = num(xm[2], -1.0)
                        bx = bx if bx >= 0 else 10.0; by = by if by >= 0 else 10.0
                        w = num(xm[3], -1.0); h = num(xm[4], -1.0)
                        if w < 0: w = W - 10.0 - bx
                        if h < 0: h = H - 28.0 - by - num(xm[0], 0.0)
                        out[-1]['rect'] = [bx, by, w, h]
                        for reg, val in (('rdx', bx + w + 10.0), ('rcx', by + h + 10.0)):
                            p = r.get(reg)
                            if p and p[0] == 'ptr': mem[p[1]] = val
                # caller-saved registers are clobbered
                for k in ['xmm%d' % i for i in range(16)]: x.pop(k, None)
                for k in ('rax', 'rcx', 'rdx', 'rsi', 'rdi', 'r8', 'r9', 'r10', 'r11'): r.pop(k, None)
    except Exception as e:
        print('# skip', hex(pc), ins, ops, e, file=sys.stderr)


INS = []
for l in open(sys.argv[1]):
    m = re.match(r'\s*([0-9a-f]+):\s+(\S+)\s*([^#]*)(#.*)?$', l.rstrip('\n'))
    if not m: continue
    pc, ins, ops, comment = int(m.group(1), 16), m.group(2), m.group(3).strip(), m.group(4) or ''
    o = [q.strip() for q in re.split(r',(?![^(]*\))', ops)] if ops else []
    INS.append((pc, ins, ops, comment, o, l))
IDX = {e[0]: i for i, e in enumerate(INS)}

# --tab VAR=VALUE: walk the control flow, deciding comparisons of VAR with its value; other
# conditional branches are explored both ways (fall-through first). Without --tab: linear sweep.
TAB = None
for a in sys.argv[4:]:
    if a.startswith('--tab='):
        k, v = a[6:].split('='); TAB = (k, int(v))
if TAB is None:
    for e in INS: exe(*e)
else:
    visited = set(); stack = [0]
    JCC = {'je': lambda c: c == 0, 'jne': lambda c: c != 0, 'jg': lambda c: c > 0, 'jge': lambda c: c >= 0,
           'jl': lambda c: c < 0, 'jle': lambda c: c <= 0, 'ja': lambda c: c > 0, 'jae': lambda c: c >= 0,
           'jb': lambda c: c < 0, 'jbe': lambda c: c <= 0}
    while stack:
        i = stack.pop()
        while 0 <= i < len(INS) and i not in visited:
            visited.add(i)
            pc, ins, ops, comment, o, l = INS[i]
            cmp_before = last_cmp
            exe(pc, ins, ops, comment, o, l)
            if ins.startswith('ret'): break
            if ins == 'jmp' and o and o[0].startswith('0x'):
                t = IDX.get(int(o[0].split()[0], 16)); i = t if t is not None else -1; continue
            if ins in JCC and o and o[0].startswith('0x'):
                t = IDX.get(int(o[0].split()[0], 16))
                if cmp_before and cmp_before[0] == TAB[0]:
                    taken = JCC[ins](TAB[1] - cmp_before[1])
                    if taken:
                        i = t if t is not None else -1; continue
                    i += 1; continue
                if t is not None: stack.append(t)
            i += 1
json.dump(out, sys.stdout, indent=None)
