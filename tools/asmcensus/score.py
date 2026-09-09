"""score.py <file.s> [...]: the MODELLED instructions per input byte at L9 for
the lazy ladder's search, per kernel shape, from the emitted assembly and the
`mfbudget` unit rates (0.297 walks, 1.725 examined candidates, 0.147 tag
skips, 0.087 fused-head resolutions per byte). Handles both forms:
  pointer  -- the walk is a separate symbol: frame = position no-match (18)
              + kernel prologue + kernel exit; paths from the kernel's loop
  inlined  -- the walk loops live inside `find_lazy`: frame = the position
              cycle through the walk (one tag test) minus the skip path
Per shape: cost = walks*frame + exams*pre + skips*skip + fused*(fused-pre).
Reads are reported beside instructions, never summed with them."""
import sys, re, os, heapq
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict3 as V
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, loop_body, spill_stats, STORE, merged

WALKS, EXAMS, SKIPS, FUSED = 0.297, 1.725, 0.147, 0.087
JCC = V.JCC


def prologue_len(blocks, succ, lab, h):
    dist = {0: 0}; pq = [(0, 0)]
    while pq:
        d, u = heapq.heappop(pq)
        if d > dist.get(u, 1e18):
            continue
        if u == h:
            return d
        b = blocks[u][1]
        for v in succ[u]:
            cut = len(b)
            for j, t in enumerate(b):
                m = JCC.match(t)
                if m and m.group(2) == lab[v]:
                    cut = j + 1
                    break
            if any(t.startswith('ret') for t in b[:cut]):
                continue
            c = d + cut
            if c < dist.get(v, 1e18):
                dist[v] = c; heapq.heappush(pq, (c, v))
    return None


def exit_len(blocks, succ, lab, h, bs):
    def seg(u, v):
        bl = blocks[u][1]; cut = len(bl)
        for j, t in enumerate(bl):
            m = JCC.match(t)
            if m and m.group(2) == lab[v]:
                cut = j + 1; break
        return bl[:cut]
    best = None
    for u in bs:
        for v in succ[u]:
            if v in bs:
                continue
            dist = {v: 0}; pq = [(0, v)]
            while pq:
                d, x = heapq.heappop(pq)
                if d > dist.get(x, 1e18):
                    continue
                bl = blocks[x][1]
                if any(t.startswith('ret') for t in bl):
                    c = d + bl.index(next(t for t in bl if t.startswith('ret'))) + 1
                    if best is None or c < best:
                        best = c
                    break
                for w in succ[x]:
                    c = d + len(seg(x, w))
                    if c < dist.get(w, 1e18):
                        dist[w] = c; heapq.heappush(pq, (c, w))
    return best


def walk_paths(blocks, succ, lab, h, bs):
    lb = loop_body(blocks, bs)
    packed = any('$16777215' in t for t in lb)
    skip = V.shortest(blocks, succ, lab, h, bs, need=4)
    miss = V.shortest(blocks, succ, lab, h, bs, need=6)
    pre = V.shortest(blocks, succ, lab, h, bs, need=38)
    fused = V.shortest(blocks, succ, lab, h, bs, need=22)
    if not (skip and miss and pre):
        return None
    wc = skip[0] < miss[0]
    return dict(packed=packed, wc=wc, skip=skip, miss=miss, pre=pre, fused=fused or pre, size=len(lb), spills=spill_stats(lb))


def shape(p):
    return ('cp' if p['packed'] else 'ca') + ('.wc' if p['wc'] else '')


def score_pointer(path):
    out = {}
    for name, pat in (('cp.wc', r'chain_find_bestKj0_Kb1_Kb0_KBX_'), ('cp', r'chain_find_bestKj0_Kb1_Kb0_KB11_'),
                      ('ca.wc', r'chain_find_bestKj0_Kb0_Kb1_KB11_'), ('ca', r'chain_find_bestKj0_Kb0_Kb1_KBX_')):
        b = symbol_bodies(path, [re.compile(pat)])
        if not b:
            continue
        sym, body = next(iter(b.items()))
        ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
        lab = {i: l for i, (l, _) in enumerate(blocks)}
        h, bs = max(loops.items(), key=lambda kv: len(loop_body(blocks, kv[1])))
        p = walk_paths(blocks, succ, lab, h, bs)
        if not p:
            continue
        frame = 18 + prologue_len(blocks, succ, lab, h) + (exit_len(blocks, succ, lab, h, bs) or 0)
        out[name] = (frame, p)
    return out


def score_inlined(path):
    b = symbol_bodies(path, [re.compile(r'encode\d+find_lazy(\b|_impl)')]); body = merged(b)
    ins = instrs(body); blocks = blocks_of(ins); loops = natural_loops(blocks); succ, pred = cfg(blocks)
    lab = {i: l for i, (l, _) in enumerate(blocks)}
    walks = []
    for h, bs in loops.items():
        lb = loop_body(blocks, bs)
        if any(re.match(r'^shrq\s+%cl', t) for t in lb):
            continue
        p = walk_paths(blocks, succ, lab, h, bs)
        if p:
            walks.append((h, bs, p))
    out = {}
    for h, bs, p in walks:
        # the smallest position loop (has the step shift) containing this walk's header
        cands = [(len(pbs), ph, pbs) for ph, pbs in loops.items() if h in pbs and any(re.match(r'^shrq\s+%cl', t) for t in loop_body(blocks, pbs))]
        if not cands:
            continue
        _, ph, pbs = min(cands)
        cyc = V.shortest(blocks, succ, lab, ph, pbs, need=12, nocall=True)
        if not cyc:
            continue
        frame = cyc[0] - p['skip'][0]
        key = shape(p)
        # two inlined sites per instance: keep the cheaper frame per shape, note both
        out.setdefault(key, []).append((frame, p, cyc, lab[h]))
    return {k: sorted(v, key=lambda t: t[0]) for k, v in out.items()}


def cost(frame, p):
    return WALKS * frame + EXAMS * p['pre'][0] + SKIPS * p['skip'][0] + FUSED * (p['fused'][0] - p['pre'][0])


for path in sys.argv[1:]:
    print(f"##### {os.path.basename(path)}")
    ptr = score_pointer(path)
    total = 0.0
    for k in ('cp.wc', 'cp', 'ca.wc', 'ca'):
        if k in ptr:
            frame, p = ptr[k]
            c = cost(frame, p)
            total += c
            print(f"  ptr {k:<6} frame {frame:>4} | skip {p['skip'][0]:>3}/{p['skip'][1]}r pre {p['pre'][0]:>3}/{p['pre'][1]}r{p['pre'][2]}s fused {p['fused'][0]:>3} | loop {p['size']:>4} sp{p['spills'][0]:>2} rd{p['spills'][1]:>3} | cost {c:6.1f}/byte")
    print(f"  ptr sum of the four shipping shapes: {total:.1f} instrs/byte (L9 model)")
    inl = score_inlined(path)
    if inl:
        total = 0.0
        for k in ('cp.wc', 'cp', 'ca.wc', 'ca'):
            for frame, p, cyc, hdr in inl.get(k, []):
                c = cost(frame, p)
                print(f"  inl {k:<6} frame {frame:>4} ({cyc[0]}/{cyc[1]}r{cyc[2]}s cycle) | skip {p['skip'][0]:>3}/{p['skip'][1]}r pre {p['pre'][0]:>3}/{p['pre'][1]}r{p['pre'][2]}s fused {p['fused'][0]:>3} | loop {p['size']:>4} sp{p['spills'][0]:>2} rd{p['spills'][1]:>3} | cost {c:6.1f}/byte  {hdr}")
            if inl.get(k):
                total += cost(inl[k][0][0], inl[k][0][1])
        print(f"  inl sum (cheapest site per shape): {total:.1f} instrs/byte (L9 model)")
