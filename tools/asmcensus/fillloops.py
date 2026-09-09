"""fillloops.py <file.s> [<file.s> ...]: the fill bodies' loops -- every
`lz_fill_range` instantiation and `row_fill_range`, each loop's size, its
mnemonics, and the per-call prologue (shortest entry->loop-header path) to
the packed hash4 quad loop. Reads the wins and the leaks the board's one-line
`F` row folds together (a producer change can move a cold arm by +12 while
the shipping quad drops 12)."""
import sys, re, heapq
import verdict3 as V
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, loop_body

PATS = [
    (r'lz_fill_rangeKb0_Kb1_KBR_KBV_', 'fill packed'),
    (r'lz_fill_rangeKb0_Kb0_Kb1_Kb1_', 'fill tag-array'),
    (r'lz_fill_rangeKb0_Kb0_Kb0_Kb1_', 'fill no-tags'),
    (r'lz_fill_rangeKb1_', 'fill rows'),
    (r'row_fill_range', 'row fill'),
]


def prologue(blocks, succ, lab, h):
    dist = {0: 0}
    pq = [(0, 0)]
    while pq:
        d, u = heapq.heappop(pq)
        if d > dist.get(u, 1e18):
            continue
        if u == h:
            return d
        bl = blocks[u][1]
        for v in succ[u]:
            cut = len(bl)
            for j, t in enumerate(bl):
                m = V.JCC.match(t)
                if m and m.group(2) == lab[v]:
                    cut = j + 1
                    break
            if any(t.startswith('ret') for t in bl[:cut]):
                continue
            c = d + cut
            if c < dist.get(v, 1e18):
                dist[v] = c
                heapq.heappush(pq, (c, v))
    return None


for path in sys.argv[1:]:
    print(f"=== {path}")
    for pat, name in PATS:
        b = symbol_bodies(path, [re.compile(pat)])
        if not b:
            continue
        for sym, body in b.items():
            ins = instrs(body)
            blocks = blocks_of(ins)
            loops = natural_loops(blocks)
            succ, _ = cfg(blocks)
            lab = {i: l for i, (l, _) in enumerate(blocks)}
            rows = sorted((len(loop_body(blocks, bs)), lab[h], h) for h, bs in loops.items())
            print(f"  {name:14} {len(ins):4} instrs | loops {[n for n, _, _ in rows]}")
            for n, l, h in rows:
                mn = ' '.join(t.split()[0] for t in loop_body(blocks, loops[h]))
                pro = prologue(blocks, succ, lab, h)
                print(f"      {l:12} n={n:3} prologue={pro} :: {mn[:150]}")
