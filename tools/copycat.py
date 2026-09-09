#!/usr/bin/env python3
"""Catalogue every byte-moving call in the emitted release asm, with LENGTHS.

WHY THE ASM AND NOT THE SOURCE
------------------------------
A source grep finds the copies you WROTE. It cannot find the ones the compiler
made, and it cannot tell you which of yours survived inlining. Only the `.s`
shows:

  * a fixed-size stack temporary the optimiser zeroes with a `memset` call,
  * a `fill` / `copy_from_slice` whose RUNTIME length made it a real call where
    a constant length would have inlined,
  * a by-value struct move or return that moves more than it looks like,
  * a copy the optimiser INTRODUCED (an argument spill, a materialised temp),
  * and, the other way, which of your written copies LLVM already inlined away
    -- so you do not "optimise" a call that does not exist.

`__rust_alloc_zeroed` is counted alongside them: it is a memset the allocator
performs on your behalf, and it is invisible to a grep for `memset`.

LENGTHS
-------
Where the length register is loaded from an immediate right before the call,
that constant is the exact byte count and is reported. Windows x64 passes
memcpy(dst=rcx, src=rdx, len=r8); SysV passes (rdi, rsi, rdx). A call with no
constant length is a RUNTIME length -- which is itself the finding, because a
constant-length copy would have been inlined instead of called.

WHAT "REDUCIBLE" MEANS
----------------------
An encoder MUST move literal bytes into its output; that traffic is the job.
The target is not "zero calls", it is "no call that moves bytes a second time,
or moves bytes nobody reads". Every candidate still has to be A/B'd -- on this
workspace's record most "obvious redundant hop" removals measure ~0.

usage: python tools/copycat.py [asm.s] [--side enc|dec|all] [--min N] [--const]
"""
import re
import sys
from collections import defaultdict

LABEL = re.compile(r"^([A-Za-z_$][\w$.@]*):")
BLOCK = re.compile(r"^(\.?L[A-Za-z0-9_$.]+):")
MEMCALL = re.compile(
    r"^\s*call[q]?\s+_?(memcpy|memmove|memset|__rust_alloc_zeroed)\b")
JMP_BACK = re.compile(r"^\s*j[a-z]{1,3}\s+(\.?L[A-Za-z0-9_$.]+)\s*$")
# Immediate into a length register, either ABI. Windows x64: r8/r8d. SysV: rdx/edx.
IMM_LEN = re.compile(
    r"^\s*mov[lq]?\s+\$(\d+),\s*%(r8d|r8|edx|rdx|esi|rsi)\b")

SIDE = [
    ("train", ("5train", "train5train", "select_fastcover", "10Dictionary")),
    ("dec", ("decode", "decompress", "10compressed", "7huffman12HuffmanTable",
             "read_table", "parse_ncount", "BitRev")),
    ("enc", ("6encode", "encode", "compress", "HuffCTable", "3fse", "7huffman",
             "2mt", "5train")),
]


def side_of(sym):
    for name, keys in SIDE:
        for k in keys:
            if k in sym:
                return name
    return "other"


def demangle(sym):
    s = re.sub(r"^_R[A-Za-z]*", "", sym)
    s = re.sub(r"C[sS][A-Za-z0-9_]{10,}_", "", s)
    s = re.sub(r"\d+(?=[a-zA-Z_])", "::", s)
    return s.replace("::::", "::")[-66:]


def main():
    args = sys.argv[1:]
    side_want, min_n, want_const = "all", 1, False
    if "--side" in args:
        k = args.index("--side")
        side_want = args[k + 1]
        del args[k:k + 2]
    if "--min" in args:
        k = args.index("--min")
        min_n = int(args[k + 1])
        del args[k:k + 2]
    if "--const" in args:
        want_const = True
        args.remove("--const")
    if args:
        path = args[0]
    else:
        import glob
        import os
        c = glob.glob("target/release/deps/rusty_zstd-*.s")
        if not c:
            sys.exit("no asm; run: cargo rustc --release -p rusty_zstd -- --emit asm")
        path = max(c, key=os.path.getmtime)

    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()

    sym_at, block_start, loop_blocks = [], {}, set()
    cur = "<none>"
    for i, ln in enumerate(lines):
        m = LABEL.match(ln)
        if m and not m.group(1).startswith(("anon.", ".L")):
            cur = m.group(1)
        b = BLOCK.match(ln)
        if b:
            block_start[b.group(1)] = i
        j = JMP_BACK.match(ln)
        if j and j.group(1) in block_start and block_start[j.group(1)] < i:
            loop_blocks.add(j.group(1))
        sym_at.append(cur)

    counts = defaultdict(lambda: defaultdict(int))
    inloop = defaultdict(int)
    consts = defaultdict(list)   # sym -> [(kind, len)]
    runtime = defaultdict(int)
    cur_block = None
    for i, ln in enumerate(lines):
        b = BLOCK.match(ln)
        if b:
            cur_block = b.group(1)
        m = MEMCALL.match(ln)
        if not m:
            continue
        kind, sym = m.group(1), sym_at[i]
        counts[sym][kind] += 1
        if cur_block in loop_blocks:
            inloop[sym] += 1
        # Walk back a few instructions for an immediate into the length reg.
        n = None
        for k in range(i - 1, max(i - 7, 0), -1):
            mm = IMM_LEN.match(lines[k])
            if mm:
                n = int(mm.group(1))
                break
            if re.match(r"^\s*call", lines[k]):
                break
        if n is None:
            runtime[sym] += 1
        else:
            consts[sym].append((kind, n))

    rows = []
    for sym, kinds in counts.items():
        sd = side_of(sym)
        if side_want != "all" and sd != side_want:
            continue
        tot = sum(kinds.values())
        if tot < min_n:
            continue
        rows.append((tot, sym, sd, kinds, inloop[sym], runtime[sym]))
    rows.sort(reverse=True)

    g = defaultdict(int)
    for _, _, _, kinds, _, _ in rows:
        for k, v in kinds.items():
            g[k] += v
    print(f"asm: {path}")
    print(f"side={side_want}  symbols={len(rows)}  memcpy={g['memcpy']} "
          f"memset={g['memset']} memmove={g['memmove']} "
          f"alloc_zeroed={g['__rust_alloc_zeroed']}")
    print(f"\n{'tot':>4} {'cpy':>4} {'set':>4} {'mov':>4} {'0al':>4} "
          f"{'loop':>5} {'rtlen':>6}  side  symbol")
    for tot, sym, sd, kinds, nl_, rt in rows:
        print(f"{tot:>4} {kinds.get('memcpy',0):>4} {kinds.get('memset',0):>4} "
              f"{kinds.get('memmove',0):>4} {kinds.get('__rust_alloc_zeroed',0):>4} "
              f"{(nl_ or ''):>5} {(rt or ''):>6}  {sd:<5} {demangle(sym)}")
    print("\n'loop'  = calls in a block something jumps BACK to; they multiply "
          "by the trip count.")
    print("'rtlen' = calls whose length is NOT a compile-time constant. A "
          "constant-length\n          copy would have been INLINED, so every "
          "one of these is a real call.")

    if want_const:
        print("\nCONSTANT-LENGTH calls (exact bytes, from the length register):")
        for _, sym, sd, _, _, _ in rows:
            if not consts[sym]:
                continue
            tot = sum(n for _, n in consts[sym])
            det = ", ".join(f"{k}:{n}" for k, n in sorted(consts[sym])[:6])
            print(f"  {tot:>8} B  {sd:<5} {demangle(sym)}\n             {det}")


if __name__ == "__main__":
    main()
