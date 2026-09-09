"""TRUE shortest header->latch path of one natural loop (Dijkstra on executed
instructions), optionally forbidding blocks that contain calls or that write
memory to a given base register, so the per-position NO-MATCH path can be
isolated. Prints the path with its stack reads and flag tests.
usage: paths2.py <file.s> <symbol-regex> <header> [nocall] [avoid=.LBBx,.LBBy]"""
import re, sys, os, collections, heapq
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cfg import symbol_bodies, instrs, blocks_of, natural_loops, cfg, STORE

bodies = symbol_bodies(sys.argv[1], [re.compile(sys.argv[2])])
sym, body = next(iter(bodies.items()))
hdr = sys.argv[3]
nocall = 'nocall' in sys.argv[4:]
avoid = set()
for a in sys.argv[4:]:
    if a.startswith('avoid='):
        avoid = set(a[6:].split(','))
ins = instrs(body)
blocks = blocks_of(ins)
loops = natural_loops(blocks)
succ, pred = cfg(blocks)
lab = {i: l for i, (l, _) in enumerate(blocks)}
h = next(i for i, l in lab.items() if l == hdr)
bs = loops[h]
latches = {u for u in bs if h in succ[u]}
JCC = re.compile(r'^j(mp|e|ne|a|ae|b|be|g|ge|l|le|s|ns|z|nz|o|no|p|np|c|nc)\s+(\.LBB\d+_\d+)')


def seg(u, v):
    """instructions of region u executed when control goes to v next"""
    b = blocks[u][1]
    cut = len(b)
    for j, t in enumerate(b):
        m = JCC.match(t)
        if m and m.group(2) == lab[v]:
            cut = j + 1
            break
    return b[:cut]


def ok_seg(u, v):
    """the executed segment of u on the way to v: no call if nocall"""
    if lab[v] in avoid:
        return False
    if nocall and any(t.startswith('call') for t in seg(u, v)):
        return False
    return True


# Dijkstra from h over loop blocks; goal = reaching h again via a latch
dist = {h: 0}
prev = {}
pq = [(0, h)]
best = None
while pq:
    d, u = heapq.heappop(pq)
    if d > dist.get(u, 1e18):
        continue
    for v in succ[u]:
        if v == h:
            if u in latches and ok_seg(u, h):
                c = d + len(seg(u, h))
                if best is None or c < best[0]:
                    best = (c, u)
            continue
        if v not in bs or not ok_seg(u, v):
            continue
        c = d + len(seg(u, v))
        if c < dist.get(v, 1e18):
            dist[v] = c
            prev[v] = u
            heapq.heappush(pq, (c, v))
if best is None:
    sys.exit("no path under the constraints")
path = [best[1]]
while path[-1] != h:
    path.append(prev[path[-1]])
path.reverse()
name = re.sub(r'^_R.*?(encode|rowfind)\d+', '', sym)[:40]
print(f"### {name} loop {hdr} shortest{' no-call' if nocall else ''} path: {best[0]} instrs over {len(path)} blocks")
reads = collections.Counter()
flags = 0
stores = 0
for k, u in enumerate(path):
    v = path[k + 1] if k + 1 < len(path) else h
    for t in seg(u, v):
        if STORE.match(t):
            stores += 1
        else:
            for mm in re.finditer(r'(-?\d+)\(%r([sb])p\)', t):
                reads[mm.group(1) + '(%r' + mm.group(2) + 'p)'] += 1
        if re.match(r'^(cmpb|testb)\s+.*-?\d+\(%r[sb]p\)', t):
            flags += 1
print(f"    stack reads {sum(reads.values())} over {len(reads)} slots, stack stores {stores}, byte-flag tests {flags}")
print("    blocks: " + ' '.join(lab[u] for u in path))
if '-v' in sys.argv:
    for k, u in enumerate(path):
        v = path[k + 1] if k + 1 < len(path) else h
        print(f"{lab[u]}:")
        for t in seg(u, v):
            print("    " + t)
