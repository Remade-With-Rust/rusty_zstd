"""The three-target verdict board: for one .s, the six chain kernels' walk
loop (shortest header->latch path = the first-word candidate path), the four
fill bodies' per-byte loops, and the lazy finder's per-position no-match loop.
usage: verdict3.py <file.s> [<file.s> ...]   (columns per file)"""
import re, sys, os, collections, heapq
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, STORE, loop_body, spill_stats, merged
JCC = re.compile(r'^j(mp|e|ne|a|ae|b|be|g|ge|l|le|s|ns|z|nz|o|no|p|np|c|nc)\s+(\.LBB\d+_\d+)')


FEAT = [re.compile(r'^callq	\*'), re.compile(r'^xorq\s'), re.compile(r'^(cmpb\s+[^$(]|cmpl\s+\$1677721[56],)'), re.compile(r'^shrq\s+%cl'), re.compile(r'^(rep\s+bsf|tzcnt|bsf)'), re.compile(r'^(cmpb\s+\(|movzbl\s+\()'), re.compile(r'^callq	_R')]   # bit4 fused-short, bit5 pre_eq, bit6 direct call   # bit3: shrq %cl (lazy_step)  bit0: indirect call, bit1: an xor (first-word / rep compare), bit2: a byte compare (the tag test)


DIRECT_CALL = re.compile(r'^callq	_R')


def shortest(blocks, succ, lab, h, bs, avoid=(), need=0, nocall=False):
    """Dijkstra over (block, features-seen) from the header back to it; every
    jump to v inside u is its own edge (a region can reach v by several jumps,
    each executing a different prefix), so a path that must execute the xor
    is found even when the region's FIRST jump to the same target precedes it."""
    latches = {u for u in bs if h in succ[u]}

    def segs(u, v):
        b = blocks[u][1]
        out = []
        for j, t in enumerate(b):
            m = JCC.match(t)
            if m and m.group(2) == lab[v]:
                out.append(b[:j + 1])
        last = b[-1] if b else ''
        ends = last.startswith(('ret', 'ud2')) or re.match(r'^jmp\w*\s', last)
        if not ends and v == u + 1:
            out.append(b)
        return out

    def feats(sg, f):
        for t in sg:
            for i, r in enumerate(FEAT):
                if r.match(t):
                    f |= 1 << i
        return f
    dist = {(h, 0): 0}; prev = {}; pq = [(0, h, 0)]; best = None
    while pq:
        d, u, f = heapq.heappop(pq)
        if d > dist.get((u, f), 1e18):
            continue
        for v in succ[u]:
            if lab[v] in avoid and v != h:
                continue
            for sg in segs(u, v):
                if nocall and any(DIRECT_CALL.match(t) for t in sg):
                    continue
                f2 = feats(sg, f)
                c = d + len(sg)
                if v == h:
                    if u in latches and (f2 & need) == need:
                        if best is None or c < best[0]:
                            best = (c, (u, f), sg)
                    continue
                if v not in bs:
                    continue
                if c < dist.get((v, f2), 1e18):
                    dist[(v, f2)] = c; prev[(v, f2)] = ((u, f), sg); heapq.heappush(pq, (c, v, f2))
    if best is None:
        return None
    segs_out = [best[2]]
    st = best[1]
    while st != (h, 0):
        pst, sg = prev[st]
        segs_out.append(sg); st = pst
    segs_out.reverse()
    reads = 0; stores = 0
    for sg in segs_out:
        for t in sg:
            if STORE.match(t):
                stores += 1
            else:
                reads += len(re.findall(r'-?\d+\(%r[sb]p\)', t))
    return best[0], reads, stores, len(segs_out), segs_out


TARGETS = [
    ('K cp.wc',   r'chain_find_bestKj0_Kb1_Kb0_KBX_'),
    ('K cp',      r'chain_find_bestKj0_Kb1_Kb0_KB11_'),
    ('K ca.wc',   r'chain_find_bestKj0_Kb0_Kb1_KB11_'),
    ('K ca',      r'chain_find_bestKj0_Kb0_Kb1_KBX_'),
    ('K none.wc', r'chain_find_bestKj0_Kb0_KBX_Kb1_'),
    ('K none',    r'chain_find_bestKj0_Kb0_KBX_KBX_'),
    ('F cp',      r'lz_fill_rangeKb0_Kb1_KBR_KBV_'),
    ('F ca',      r'lz_fill_rangeKb0_KBR_Kb1_KBZ_'),
    ('F none',    r'lz_fill_rangeKb0_KBR_KBR_Kb1_'),
    ('F rows',    r'lz_fill_rangeKb1_Kb0_KBV_KBV_'),
    ('P lazy',    r'encode\d+find_lazy(\b|_impl)'),
    ('P greedy',  r'encode\d+find_greedy(\b|_impl)'),
]


def one(path):
    out = {}
    for name, pat in TARGETS:
        b = symbol_bodies(path, [re.compile(pat)])
        if not b:
            out[name] = None; continue
        body = merged(b)
        ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
        lab = {i: l for i, (l, _) in enumerate(blocks)}
        rows = []
        for h, bs in loops.items():
            lb = loop_body(blocks, bs); sp, rl = spill_stats(lb)
            rows.append((len(lb), h, bs, sp, rl))
        if name.startswith('K'):
            # the walk loop: the largest loop
            n, h, bs, sp, rl = max(rows)
            r = shortest(blocks, succ, lab, h, bs)
            tagged = 'none' not in name
            # prologue: Dijkstra from block 0 to the loop header over non-loop blocks
            pro = None
            dist = {0: 0}; pq = [(0, 0)]
            import heapq as _hq
            while pq:
                d, u = _hq.heappop(pq)
                if d > dist.get(u, 1e18):
                    continue
                if u == h:
                    pro = d; break
                b = blocks[u][1]
                for v in succ[u]:
                    # executed prefix of u up to the jump to v (or whole block on fall-through)
                    cut = len(b)
                    for j, t in enumerate(b):
                        m_ = JCC.match(t)
                        if m_ and m_.group(2) == lab[v]:
                            cut = j + 1; break
                    if any(t.startswith('ret') for t in b[:cut]):
                        continue
                    c = d + cut
                    if c < dist.get(v, 1e18):
                        dist[v] = c; _hq.heappush(pq, (c, v))
            base = 4 if tagged else 0
            r = shortest(blocks, succ, lab, h, bs, need=base)
            rx = shortest(blocks, succ, lab, h, bs, need=base | 2)
            rp = shortest(blocks, succ, lab, h, bs, need=base | 2 | 32)
            rf = shortest(blocks, succ, lab, h, bs, need=base | 2 | 16)
            out[name] = (len(ins), lab[h], n, sp, rl, r, rx, pro, rp, rf)
        elif name.startswith('F'):
            # every per-byte loop, smallest first: (loop instrs, path)
            rows.sort()
            out[name] = (len(ins), [(lab[h], n, sp, rl, shortest(blocks, succ, lab, h, bs)) for n, h, bs, sp, rl in rows])
        else:
            # the per-position loop: the loop whose header holds the kernel indirect call...
            # take every loop containing an indirect call and report the smallest path
            best = None
            for n, h, bs, sp, rl in rows:
                lb = loop_body(blocks, bs)
                if not any(t.startswith('callq\t*') for t in lb):
                    continue
                # avoid the emit blocks: those calling push_literals / grow / fill
                r = shortest(blocks, succ, lab, h, bs, need=8, nocall=True)
                r3 = shortest(blocks, succ, lab, h, bs, need=10, nocall=True)
                if r and (best is None or r[0] < best[1][0]):
                    best = (lab[h], r, n, sp, rl, r3)
            # inlined walk loops (brick 58+): loops with the tag test and the first-word xor, no shift
            walks = []
            for n, h, bs, sp, rl in rows:
                lb = loop_body(blocks, bs)
                if any(re.match(r'^shrq\s+%cl', t) for t in lb):
                    continue
                r6 = shortest(blocks, succ, lab, h, bs, need=6)
                rp = shortest(blocks, succ, lab, h, bs, need=38)
                if r6:
                    walks.append((r6[0], r6[1], r6[2], n, sp, rl, rp))
            walks.sort()
            # the look-ahead step: a loop with the indirect call and WITHOUT the step shift
            look = None
            for n, h, bs, sp, rl in rows:
                lb = loop_body(blocks, bs)
                if not any(t.startswith('callq	*') for t in lb) or any(re.match(r'^shrq\s+%cl', t) for t in lb):
                    continue
                r = shortest(blocks, succ, lab, h, bs, need=1, nocall=True)
                if r and (look is None or r[0] < look[0]):
                    look = r
            out[name] = (len(ins), best, look, walks)
    return out


def dump(path, target, need):
    for name, pat in TARGETS:
        if name != target:
            continue
        b = symbol_bodies(path, [re.compile(pat)])
        body = merged(b)
        ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
        lab = {i: l for i, (l, _) in enumerate(blocks)}
        best = None
        for h, bs in loops.items():
            r = shortest(blocks, succ, lab, h, bs, need=need, nocall=name.startswith('P'))
            if r and (best is None or r[0] < best[0]):
                best = r
        print(f'### {target} need={need}: {best[0]} instrs, {best[1]} reads, {best[2]} stores')
        for sg in best[4]:
            print('  --')
            for t in sg:
                print('    ' + t)


if __name__ == '__main__':
    if len(sys.argv) > 3 and sys.argv[2] in dict(TARGETS):
        dump(sys.argv[1], sys.argv[2], int(sys.argv[3]))
        sys.exit()
    cols = [one(p) for p in sys.argv[1:]]
    print("target      " + "".join(f"{os.path.basename(p)[:22]:>60}" for p in sys.argv[1:]))
    print("K: total | prologue | walk loop | paths: tag-skip, first-word MISS, first-word pass + pre_eq fail (the 83% path at L9), pass + fused short;  F: per-byte loops;  P: per-position no-match path without / with the rep probe")
    for name, _ in TARGETS:
        line = f"{name:<12}"
        for c in cols:
            v = c.get(name)
            if v is None:
                line += f"{'-':>42}"; continue
            if name.startswith('K'):
                n, hdr, ln, sp, rl, r, rx, pro, rp, rf = v
                fmt = lambda q: f'{q[0]}/{q[1]}r{q[2]}s' if q else '-'
                line += f"{f'{n} | pro {pro} | L{ln} sp{sp} rd{rl} | skip {fmt(r)} miss {fmt(rx)} pre {fmt(rp)} fused {fmt(rf)}':>60}"
            elif name.startswith('F'):
                n, rows = v
                line += f"{f'{n} | ' + ' '.join(f'{r[0] if r else 0}/{r[1] if r else 0}r' for _, _, _, _, r in rows):>42}"
            else:
                n, best, look, walks = v
                if best is None:
                    line += f"{f'{n} | no loop':>42}"
                else:
                    hdr, r, ln, sp, rl, r3 = best
                    lk = f' look {look[0]}/{look[1]}r' if look else ' look -'
                    if walks:
                        lk += ' walk miss ' + ' '.join(f'{w[0]}/{w[1]}r' for w in walks[:2]) + ' pre ' + ' '.join(f'{w[6][0]}/{w[6][1]}r' if w[6] else '-' for w in walks[:2])
                    line += f"{(f'{n} | norep {r[0]}/{r[1]}r{r[2]}s rep {r3[0]}/{r3[1]}r{r3[2]}s' if r3 else f'{n} | norep {r[0]}/{r[1]}r{r[2]}s') + lk:>42}"
        print(line)
