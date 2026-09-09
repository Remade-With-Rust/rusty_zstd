"""pathdump2.py <file.s> <symbol-regex> <header> <need> [nocall]: the shortest
header->latch cycle of that loop executing the `need` features (bit0 indirect
call, bit1 xor, bit2 tag byte-compare, bit3 lazy_step shift, bit4 fused bsf,
bit5 pre_eq memory cmpb), printed segment by segment."""
import sys, re, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict3 as V
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg
b = symbol_bodies(sys.argv[1], [re.compile(sys.argv[2])]); sym, body = next(iter(b.items()))
ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
lab = {i: l for i, (l, _) in enumerate(blocks)}
h = next(i for i, l in lab.items() if l == sys.argv[3])
r = V.shortest(blocks, succ, lab, h, loops[h], need=int(sys.argv[4]), nocall='nocall' in sys.argv)
if not r:
    sys.exit("no path")
print(f"### {sys.argv[3]} need={sys.argv[4]}: {r[0]} instrs, {r[1]} reads, {r[2]} stores")
for sg in r[4]:
    print("  --")
    for t in sg:
        print("    " + t)
