"""Per-loop census with a CONTENT signature, so loops can be matched across builds.
usage: loopsig.py <file.s> <symbol-regex> [<symbol-regex> ...]"""
import re, sys, collections

L = open(sys.argv[1], encoding='utf-8', errors='replace').read().split('\n')
pats = [re.compile(p) for p in sys.argv[2:]]
cur = None
bodies = collections.OrderedDict()
for l in L:
    m = re.match(r'^(_R[A-Za-z0-9_]+):', l)
    if m:
        cur = m.group(1)
        continue
    if cur and any(p.search(cur) for p in pats):
        bodies.setdefault(cur, []).append(l.strip())
JT = re.compile(r'^j\w+\s+(\.LBB\d+_\d+)')
for sym, b in bodies.items():
    ins = [t for t in b if t and not t.startswith('#') and (not t.startswith('.') or re.match(r'^\.LBB\d+_\d+:', t))]
    labels = {}
    flat = []
    for t in ins:
        m = re.match(r'^(\.LBB\d+_\d+):', t)
        if m:
            labels[m.group(1)] = len(flat)
            continue
        flat.append(t)
    short = re.sub(r'^_R.*?(encode|rowfind)\d+', '', sym)[:40]
    print(f"##### {short}: {len(flat)} instrs")
    rows = []
    for i, t in enumerate(flat):
        m = JT.match(t)
        if m and m.group(1) in labels and labels[m.group(1)] <= i:
            a = labels[m.group(1)]
            seg = flat[a:i + 1]
            if any(x.startswith('ret') for x in seg):
                continue
            rl = sum(len(re.findall(r'-?\d+\(%r[sb]p\)', x)) for x in seg)
            sp = sum(1 for x in seg if re.match(r'^mov\w*\s+%\w+,\s*-?\d+\(%r[sb]p\)', x))
            inner = sum(1 for x in seg[:-1] if JT.match(x) and JT.match(x).group(1) in labels and a <= labels[JT.match(x).group(1)] < i)
            calls = [re.sub(r'^call\w*\s+', '', x) for x in seg if x.startswith('call')]
            calls = [re.sub(r'.*?(encode|rowfind|simd)\d+', '', c)[:18] if not c.startswith('*') else 'INDIRECT' for c in calls]
            consts = collections.Counter(re.findall(r'\$(-?\d{6,})', ' '.join(seg)))
            stores = sum(1 for x in seg if re.match(r'^mov[bwlq]\s+%\w+,\s*\(', x))
            rows.append((len(seg), sp, rl - sp, inner, m.group(1), ','.join(calls[:4]), ' '.join(f"{k[:6]}x{v}" for k, v in consts.most_common(3)), stores))
    rows.sort()
    for n, sp, rl, inner, lab, calls, consts, st in rows:
        print(f"  {lab:<12} {n:>4}i {sp:>2}s {rl:>3}r inner={inner:<2} stores={st:<2} calls=[{calls}] consts={consts}")
