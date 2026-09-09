"""btpos.py <file.s>: the bt-lazy finder's position loops, no-match cycle with
the kernel call allowed (its search is a call)."""
import sys, re, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict3 as V
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, loop_body
b = symbol_bodies(sys.argv[1], [re.compile(r'encode\d+find_bt_lazy\b')])
for sym, body in b.items():
    ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
    lab = {i: l for i, (l, _) in enumerate(blocks)}
    out = [f"{os.path.basename(sys.argv[1])} find_bt_lazy {len(ins)}"]
    for h, bs in sorted(loops.items(), key=lambda kv: len(kv[1])):
        lb = loop_body(blocks, bs)
        if not any(re.match(r'^shrq\s+%cl', t) for t in lb):
            continue
        r = V.shortest(blocks, succ, lab, h, bs, need=8)
        out.append(f"  position {lab[h]} L{len(lb)} no-match {r[0]}/{r[1]}r{r[2]}s" if r else f"  position {lab[h]} L{len(lb)} -")
    print("\n".join(out))
