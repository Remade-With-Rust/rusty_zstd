"""gwalk.py <file.s> [...]: the greedy finder's inline walk and frame -- the
dominant (pre_eq-fail) path, the tag-skip path, and the position cycle
through the walk -- for the L5 ports of the lazy walk's bricks."""
import sys, re, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict3 as V
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, loop_body, spill_stats, merged
for path in sys.argv[1:]:
    b = symbol_bodies(path, [re.compile(r'encode\d+find_greedy(\b|_impl)')]); body = merged(b)
    ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
    lab = {i: l for i, (l, _) in enumerate(blocks)}
    out = [f"{os.path.basename(path)} find_greedy {len(ins)}"]
    for h, bs in sorted(loops.items(), key=lambda kv: len(kv[1])):
        lb = loop_body(blocks, bs); sp, rl = spill_stats(lb)
        feats = set(i for t in lb for i, r in enumerate(V.FEAT) if r.match(t))
        f = lambda q: f"{q[0]}/{q[1]}r{q[2]}s" if q else "-"
        if 1 in feats and 2 in feats and 3 not in feats:
            skip = V.shortest(blocks, succ, lab, h, bs, need=4); pre = V.shortest(blocks, succ, lab, h, bs, need=38); fused = V.shortest(blocks, succ, lab, h, bs, need=22)
            out.append(f"  walk {lab[h]} L{len(lb)} sp{sp} rd{rl} | skip {f(skip)} pre {f(pre)} fused {f(fused)}")
        elif 3 in feats:
            r = V.shortest(blocks, succ, lab, h, bs, need=8, nocall=True); rw = V.shortest(blocks, succ, lab, h, bs, need=12, nocall=True)
            if rw:
                out.append(f"  position {lab[h]} L{len(lb)} | no-match {f(r)} | through walk {f(rw)}")
    print("\n".join(out))
