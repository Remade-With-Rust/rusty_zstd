"""Hot stack slots of ONE natural loop (dominance-based), with provenance.
usage: hotslots2.py <file.s> <symbol-regex> <loop-header-label|auto> [top-n]
`auto` = the smallest natural loop (>= 60 instrs) that calls count_match_raw."""
import re, sys, collections, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, loop_body, STORE, cfg, dominators

bodies = symbol_bodies(sys.argv[1], [re.compile(sys.argv[2])])
sym, body = next(iter(bodies.items()))
want = sys.argv[3]
top = int(sys.argv[4]) if len(sys.argv) > 4 else 24
ins = instrs(body)
blocks = blocks_of(ins)
loops = natural_loops(blocks)
lab_of = {i: l for i, (l, _) in enumerate(blocks)}
if want == 'auto':
    cands = []
    for h, bs in loops.items():
        lb = loop_body(blocks, bs)
        if len(lb) >= 60 and any(t.startswith('call') and 'count_match_raw' in t for t in lb):
            cands.append((len(lb), h))
    cands.sort()
    h = cands[0][1]
else:
    h = next(i for i, l in lab_of.items() if l == want)
bs = loops[h]
lb = loop_body(blocks, bs)
# latches: blocks in the loop with an edge to h
succ, pred = cfg(blocks)
latches = [u for u in bs if h in succ[u]]
reads = collections.Counter()
writes_in = collections.Counter()
for t in lb:
    ms = STORE.match(t)
    if ms:
        writes_in[ms.group(1) + '(%r' + ms.group(2) + 'p)'] += 1
        continue
    for mm in re.finditer(r'(-?\d+)\(%r([sb])p\)', t):
        reads[mm.group(1) + '(%r' + mm.group(2) + 'p)'] += 1
first_store = {}
for j, t in enumerate(ins):
    ms = STORE.match(t)
    if ms:
        k = ms.group(1) + '(%r' + ms.group(2) + 'p)'
        if k not in first_store:
            ctx = [x for x in ins[max(0, j - 3):j] if not x.startswith('.LBB')]
            first_store[k] = ' | '.join(ctx + [t])
in_store = {}
for t in lb:
    ms = STORE.match(t)
    if ms:
        k = ms.group(1) + '(%r' + ms.group(2) + 'p)'
        in_store.setdefault(k, t)
name = re.sub(r'^_R.*?(encode|rowfind)\d+', '', sym)[:40]
nsp = sum(writes_in.values())
nrd = sum(reads.values())
print(f"### {name} loop {lab_of[h]}: {len(lb)} instrs, {len(bs)} blocks, latches={[lab_of[u] for u in latches]}, {nsp} spill stores, {nrd} stack reads, {len(reads)} distinct slots")
print(f"{'slot':<12}{'reads':>6}{'w-in':>5}  kind      provenance")
for k in sorted(set(list(reads) + list(writes_in)), key=lambda k: -(reads[k] + writes_in[k]))[:top]:
    kind = 'SPILL' if writes_in[k] else ('HOISTED' if k in first_store else 'INCOMING')
    prov = in_store.get(k, first_store.get(k, '-')) if writes_in[k] else first_store.get(k, '-')
    print(f"{k:<12}{reads[k]:>6}{writes_in[k]:>5}  {kind:<9} {prov[:140]}")
