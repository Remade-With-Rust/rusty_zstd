"""Print the instructions executed along an explicit block path through a loop
(each region up to the jump that is TAKEN to reach the next block), and tally
its stack reads by slot and the byte-flag tests on it.
usage: pathdump.py <file.s> <symbol-regex> <header> <block> <block> ..."""
import re, sys, os, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cfg import symbol_bodies, instrs, blocks_of, STORE

bodies = symbol_bodies(sys.argv[1], [re.compile(sys.argv[2])])
sym, body = next(iter(bodies.items()))
ins = instrs(body)
blocks = blocks_of(ins)
idx = {l: i for i, (l, _) in enumerate(blocks)}
path = [idx[x] for x in sys.argv[3:]]
JCC = re.compile(r'^j(mp|e|ne|a|ae|b|be|g|ge|l|le|s|ns|z|nz|o|no|p|np|c|nc)\s+(\.LBB\d+_\d+)')
h = path[0]
tot = 0
reads = collections.Counter()
flags = 0
for k, i in enumerate(path):
    b = blocks[i][1]
    nxt = path[k + 1] if k + 1 < len(path) else h
    cut = len(b)
    for j, t in enumerate(b):
        m = JCC.match(t)
        if m and m.group(2) == blocks[nxt][0]:
            cut = j + 1
            break
    seg = b[:cut]
    print(f"{blocks[i][0]}:")
    for t in seg:
        mark = ''
        if not STORE.match(t):
            for mm in re.finditer(r'(-?\d+)\(%r([sb])p\)', t):
                reads[mm.group(1) + '(%r' + mm.group(2) + 'p)'] += 1
                mark = '   <-- stack read'
        if re.match(r'^(cmpb|testb)\s+\$?\d*,?\s*-?\d+\(%r[sb]p\)', t) or re.match(r'^cmpb\s+\$0,\s*-?\d+\(%r[sb]p\)', t):
            flags += 1
            mark = '   <-- FLAG test'
        print(f"    {t}{mark}")
    tot += len(seg)
print(f"=== path: {tot} instrs, {sum(reads.values())} stack reads over {len(reads)} slots, {flags} byte-flag tests")
print("    slots: " + ', '.join(f"{k} x{v}" for k, v in reads.most_common()))
