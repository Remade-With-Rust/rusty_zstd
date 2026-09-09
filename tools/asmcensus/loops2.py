"""Dominance-based natural-loop census per symbol: instrs / spill stores /
stack reads / rip-relative static loads / calls, plus the statics named.
usage: loops2.py <file.s> <symbol-regex> [...]"""
import re, sys, collections, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, loop_body, spill_stats

bodies = symbol_bodies(sys.argv[1], [re.compile(p) for p in sys.argv[2:]])
RIP = re.compile(r'_R[A-Za-z0-9_]*?(?:encode|lib|prof|rowfind|ldm|simd)\d+([A-Za-z0-9_]+?)(?:\.0)?\(%rip\)')
for sym, b in bodies.items():
    ins = instrs(b)
    blocks = blocks_of(ins)
    loops = natural_loops(blocks)
    name = re.sub(r'^_R.*?(encode|rowfind|ldm)\d+', '', sym)[:38]
    n_all = sum(len(x) for _, x in blocks)
    print(f"##### {name}: {n_all} instrs, {len(blocks)} blocks, {len(loops)} natural loops")
    rows = []
    for h, bs in loops.items():
        lb = loop_body(blocks, bs)
        sp, rl = spill_stats(lb)
        rips = collections.Counter(m.group(1)[:24] for t in lb for m in RIP.finditer(t))
        calls = collections.Counter()
        for t in lb:
            if t.startswith('call'):
                c = re.sub(r'^call\w*\s+', '', t)
                c = 'INDIRECT' if c.startswith('*') else re.sub(r'.*?(encode|rowfind|simd|ldm|alloc|core|std)\d+', '', c)[:22]
                calls[c] += 1
        rows.append((len(lb), blocks[h][0], len(bs), sp, rl, rips, calls))
    rows.sort(key=lambda r: -r[0])
    print(f"  {'header':<12}{'instrs':>7}{'blks':>5}{'spills':>7}{'reads':>6}{'rip':>4}  calls")
    for n, lab, nb, sp, rl, rips, calls in rows[:10]:
        cs = ' '.join(f"{k}x{v}" if v > 1 else k for k, v in calls.most_common(5))
        print(f"  {lab:<12}{n:>7}{nb:>5}{sp:>7}{rl:>6}{sum(rips.values()):>4}  {cs[:80]}")
        if rips:
            print("      statics: " + ', '.join(f"{k} x{v}" for k, v in rips.most_common(10)))
