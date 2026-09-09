"""reppath.py <file.s>: the lazy finder's inlined position loops -- the no-match
cycle with the rep probe executed (need: shift + xor, no direct call)."""
import sys, re, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict3 as V
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, loop_body, merged
b = symbol_bodies(sys.argv[1], [re.compile(r'encode\d+find_lazy(\b|_impl)')]); body = merged(b)
ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
lab = {i: l for i, (l, _) in enumerate(blocks)}
rows = []
for h, bs in loops.items():
    lb = loop_body(blocks, bs)
    if not any(re.match(r'^shrq\s+%cl', t) for t in lb) or any(t.startswith('callq\t*') for t in lb):
        continue
    r = V.shortest(blocks, succ, lab, h, bs, need=8, nocall=True)
    rr = V.shortest(blocks, succ, lab, h, bs, need=10, nocall=True)
    if r and rr:
        rows.append((len(lb), lab[h], r, rr))
rows.sort()
f = lambda q: f"{q[0]}/{q[1]}r{q[2]}s"
print(os.path.basename(sys.argv[1]), "inlined position loops: no-match | with the rep probe")
for n, l, r, rr in rows[:6]:
    print(f"  L{n:<4} {l:<12} {f(r):>10} | {f(rr):>10}")
