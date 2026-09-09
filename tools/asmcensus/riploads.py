"""rip-relative (static / knob-arm) loads inside each finder's natural loops:
which statics, how many times, in which loop (size)."""
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
    if cur and any(p.search(cur) for p in pats) and 'bmi2' not in cur:
        bodies.setdefault(cur, []).append(l.strip())
JCC = re.compile(r'^j(mp|e|ne|a|ae|b|be|g|ge|l|le|s|ns|z|nz|o|no|p|np|c|nc)\s+(\.LBB\d+_\d+)')


def natloops(ins):
    blocks = []
    lab = 'ENTRY'
    curb = []
    for t in ins:
        m = re.match(r'^(\.LBB\d+_\d+):', t)
        if m:
            blocks.append((lab, curb))
            lab = m.group(1)
            curb = []
            continue
        curb.append(t)
    blocks.append((lab, curb))
    idx = {l: i for i, (l, _) in enumerate(blocks)}
    succ = collections.defaultdict(set)
    for i, (l, b) in enumerate(blocks):
        last = b[-1] if b else ''
        m = JCC.match(last)
        if m:
            if m.group(2) in idx:
                succ[i].add(idx[m.group(2)])
            if m.group(1) != 'mp' and i + 1 < len(blocks):
                succ[i].add(i + 1)
        elif last.startswith(('ret', 'ud2', 'jmp')):
            pass
        elif i + 1 < len(blocks):
            succ[i].add(i + 1)
    pred = collections.defaultdict(set)
    for u, vs in succ.items():
        for v in vs:
            pred[v].add(u)
    loops = {}
    for u, vs in succ.items():
        for h in vs:
            if h <= u:
                bs = {h, u}
                st = [u]
                while st:
                    x = st.pop()
                    for q in pred[x]:
                        if q not in bs:
                            bs.add(q)
                            st.append(q)
                loops[h] = loops.get(h, set()) | bs
    return blocks, loops


for sym, b in bodies.items():
    ins = [t for t in b if t and not t.startswith('#') and (not t.startswith('.') or re.match(r'^\.LBB\d+_\d+:', t))]
    blocks, loops = natloops(ins)
    name = re.sub(r'^_R.*?(encode|rowfind)\d+', '', sym)[:36]
    rows = []
    for h, bs in loops.items():
        lb = [t for i in sorted(bs) for t in blocks[i][1]]
        rips = collections.Counter()
        for t in lb:
            for mm in re.finditer(r'_R[A-Za-z0-9_]*?encode\d+([A-Za-z0-9_]+?)\.0\(%rip\)|_R[A-Za-z0-9_]*?(?:encode|lib|prof|rowfind|ldm)\d+([A-Za-z0-9_]+)\(%rip\)', t):
                rips[(mm.group(1) or mm.group(2))[:28]] += 1
        calls = sum(1 for t in lb if t.startswith('call'))
        rows.append((len(lb), blocks[h][0], sum(rips.values()), rips, calls, any('count_match_raw' in t or 'call\t*' in t or 'callq\t*' in t for t in lb)))
    rows.sort(key=lambda r: -r[0])
    print(f"##### {name}: {len(loops)} natural loops")
    for n, lab, nr, rips, calls, hot in rows[:8]:
        if nr == 0:
            print(f"  {lab:<12} {n:>5} instrs  rip-loads=0")
            continue
        print(f"  {lab:<12} {n:>5} instrs  rip-loads={nr:<3} calls={calls:<3} {'[per-position]' if hot else ''}")
        print("      " + ', '.join(f"{k} x{v}" for k, v in rips.most_common(14)))
