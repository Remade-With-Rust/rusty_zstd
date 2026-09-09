"""Shared: parse one symbol's body into basic blocks and find its NATURAL loops
using dominators (a back edge u->h counts only when h dominates u), so a jump
to an earlier-laid-out block that is not a loop header no longer creates a
phantom loop spanning the prologue."""
import re, collections

JCC = re.compile(r'^j(mp|e|ne|a|ae|b|be|g|ge|l|le|s|ns|z|nz|o|no|p|np|c|nc)\s+(\.LBB\d+_\d+)')


JUMP_TABLES = {}


def symbol_bodies(path, pats, skip_bmi2=True):
    """also harvests every jump table (.LJTIn_m: .long .LBBn_x-.LJTIn_m) so
    `jmpq *reg` blocks get their real successors"""
    L = open(path, encoding='utf-8', errors='replace').read().split('\n')
    cur = None
    bodies = collections.OrderedDict()
    jt = None
    for l in L:
        t = l.strip()
        mj = re.match(r'^(\.LJTI\d+_\d+):', t)
        if mj:
            jt = mj.group(1)
            JUMP_TABLES[jt] = []
            continue
        if jt:
            me = re.match(r'^\.(?:long|quad|rva)\s+(\.LBB\d+_\d+)', t)
            if me:
                JUMP_TABLES[jt].append(me.group(1))
                continue
            jt = None
        # a function label: Rust v0 (`_R...`) or a plain C symbol; never `.LBB`/`.L`
        m = re.match(r'^([A-Za-z_][A-Za-z0-9_$.]*):', l)
        if m and not m.group(1).startswith(('.', 'Lfunc', 'Ltmp', 'Lexception')):
            cur = m.group(1)
            continue
        if cur and any(p.search(cur) for p in pats) and not (skip_bmi2 and 'bmi2' in cur):
            bodies.setdefault(cur, []).append(t)
    return bodies


def merged(bodies):
    """the bodies of every matched symbol as ONE instruction list, each behind a
    synthetic entry label so it starts its own block (BRICK 103 outlined the
    lazy finder's five KIND shapes into five symbols; the loop metrics are
    per loop, so a union is what the board wants)"""
    out = []
    for k, body in enumerate(bodies.values()):
        out.append(f'.LBB9999_{k}:')
        out.extend(body)
    return out


def instrs(body):
    return [t for t in body if t and not t.startswith('#') and (not t.startswith('.') or re.match(r'^\.LBB\d+_\d+:', t))]


def blocks_of(ins):
    blocks = []
    lab = 'ENTRY'
    curb = []
    for t in ins:
        m = re.match(r'^(\.LBB\d+_\d+):', t)
        if m:
            blocks.append((lab, curb))
            lab = m.group(1)
            curb = []
            continue
        curb.append(t)
    blocks.append((lab, curb))
    return blocks


def cfg(blocks):
    idx = {l: i for i, (l, _) in enumerate(blocks)}
    succ = collections.defaultdict(set)
    # A label-delimited region can hold SEVERAL conditional jumps (LLVM emits a
    # label only where something jumps to), so every jump in the region is an
    # edge; the fall-through edge exists unless the region ends unconditionally.
    for i, (l, b) in enumerate(blocks):
        for j, t in enumerate(b):
            m = JCC.match(t)
            if m:
                if m.group(2) in idx:
                    succ[i].add(idx[m.group(2)])
            elif re.match(r'^jmp\w*\s+\*', t):
                tbl = None
                for x in reversed(b[max(0, j - 8):j]):
                    mt = re.search(r'(\.LJTI\d+_\d+)', x)
                    if mt:
                        tbl = mt.group(1)
                        break
                for tgt in JUMP_TABLES.get(tbl, []):
                    if tgt in idx:
                        succ[i].add(idx[tgt])
        last = b[-1] if b else ''
        ends = last.startswith(('ret', 'ud2')) or re.match(r'^jmp\w*\s', last)
        if not ends and i + 1 < len(blocks):
            succ[i].add(i + 1)
    pred = collections.defaultdict(set)
    for u, vs in succ.items():
        for v in vs:
            pred[v].add(u)
    return succ, pred


def dominators(n, succ, pred):
    # iterative data-flow: dom[v] = {v} U intersect(dom[p] for p in pred[v]); entry = 0
    full = set(range(n))
    dom = [full.copy() for _ in range(n)]
    dom[0] = {0}
    changed = True
    while changed:
        changed = False
        for v in range(1, n):
            ps = [dom[p] for p in pred[v]]
            new = ({v} | set.intersection(*ps)) if ps else {v}
            if new != dom[v]:
                dom[v] = new
                changed = True
    return dom


def natural_loops(blocks):
    """returns {header_index: set(block indices)} for real back edges only"""
    succ, pred = cfg(blocks)
    dom = dominators(len(blocks), succ, pred)
    loops = {}
    for u, vs in succ.items():
        for h in vs:
            if h in dom[u]:  # h dominates u: a real back edge
                bs = {h, u}
                st = [] if u == h else [u]
                while st:
                    x = st.pop()
                    for q in pred[x]:
                        if q not in bs:
                            bs.add(q)
                            if q != h:  # never walk past the header
                                st.append(q)
                loops[h] = loops.get(h, set()) | bs
    return loops


def loop_body(blocks, bs):
    return [t for i in sorted(bs) for t in blocks[i][1]]


STORE = re.compile(r'^mov\w*\s+[^,]+,\s*(-?\d+)\(%r([sb])p\)$')


def spill_stats(lb):
    sp = sum(1 for t in lb if STORE.match(t))
    rl = sum(len(re.findall(r'-?\d+\(%r[sb]p\)', t)) for t in lb if not STORE.match(t))
    return sp, rl
