"""poscycle.py <file.s>: the lazy finder's per-POSITION cycles, priced two ways:
the plain no-match cycle (need: the lazy_step shift; no direct call) and the
cycle THROUGH THE WALK (also executes a tag compare, i.e. at least one
candidate) -- for an inlined finder the second is position + walk entry +
one candidate + walk exit, which for the pointer-dispatched finder is
position (18) + kernel prologue + candidate + kernel exit."""
import sys, re, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict3 as V
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, loop_body, spill_stats, merged
path = sys.argv[1]
b = symbol_bodies(path, [re.compile(r'encode\d+find_lazy(\b|_impl)')]); body = merged(b)
ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
lab = {i: l for i, (l, _) in enumerate(blocks)}
rows = []
for h, bs in loops.items():
    lb = loop_body(blocks, bs)
    if not any(re.match(r'^shrq\s+%cl', t) for t in lb):
        continue
    has_ind = any(t.startswith('callq\t*') for t in lb)
    r = V.shortest(blocks, succ, lab, h, bs, need=8, nocall=True)
    rw = V.shortest(blocks, succ, lab, h, bs, need=12, nocall=True)
    rwx = V.shortest(blocks, succ, lab, h, bs, need=14, nocall=True)   # + the first-word xor: through a candidate that is examined
    rows.append((len(lb), lab[h], 'ptr' if has_ind else 'inl', r, rw, rwx))
rows.sort()
print(os.path.basename(path), "find_lazy", len(ins), "instrs")
print("  loop-size header kind | no-match cycle | cycle through the walk (>=1 tag test) | ... and an examined candidate")
for n, l, k, r, rw, rwx in rows:
    f = lambda q: f"{q[0]}/{q[1]}r{q[2]}s" if q else "-"
    print(f"  {n:>4} {l:<12} {k} | {f(r):>10} | {f(rw):>10} | {f(rwx):>10}")
