#!/usr/bin/env python3
"""Find the four mechanically-detectable inefficiencies in innermost loops.

Reading the emitted assembly of a hot function and asking what each instruction
is FOR is the only instrument that finds this class -- each costs 2-10
instructions, so nothing profiles as a hotspot and a reviewer walks past all of
them. These four patterns need no judgement at all, which is exactly why they
find things a careful reader misses:

  HALF-STORE  a store narrower than the loop's widest register (a `movq %xmm`,
              or a 128-bit store out of a `ymm`). The loop is doing half the
              work its registers are shaped for; pairing two groups halves the
              pack+store overhead.

  NARROW      a small outputs-per-trip count next to a much larger one in the
              same kernel -- a tail that could step wider, or share loads with
              its neighbour.

  INVARIANT   a `set1`/`vpbroadcast`/constant load INSIDE the loop body. It does
              not depend on the induction variable and belongs above it.

  SELECT      three or more `pcmpeq` in one body: a compare/mask/merge chain a
              single `pshufb` table lookup replaces.

CAVEAT THAT MUST TRAVEL WITH THE OUTPUT. These are CANDIDATES, not findings. A
`pshufb` rewrite that removes ALU ops but not per-trip overhead can still lose
on a short body -- one measured 3.125 instructions per output against the
compare-chain it replaced at 2.812, and only won (2.188) once the trips were
paired. Price every candidate at its paired width before believing it.

usage: python tools/loopscan.py [asm.s] [--sym REGEX] [--min-trip N]
"""
import re
import sys

LABEL = re.compile(r"^(\.?L[A-Za-z0-9_$.]+|[A-Za-z_$][\w$.@]*):")
JMP_ANY = re.compile(r"^\s*(j[a-z]{1,3})\s+(\.?L[A-Za-z0-9_$.]+)\s*$")
DIRECTIVE = re.compile(r"^\s*\.")
YMM = re.compile(r"%ymm\d+")
XMM = re.compile(r"%xmm\d+")
# stores: mnemonic then a memory destination as the LAST operand
STORE = re.compile(r"^\s*(v?mov[a-z]*|v?movnt[a-z]*)\s+(%[a-z0-9]+),\s*[-\d(]")
# INVARIANT must be UNAMBIGUOUS or it manufactures findings. The first version
# also matched `pshufd $0, %xmm, %xmm`, which broadcasts a value just LOADED in
# the body -- loop-VARIANT -- and it flagged three loops whose real constants
# LLVM had already hoisted above the backedge. Only a load from a rip-relative
# constant INSIDE the body is genuinely invariant, so that is all this matches
# now. A detector that fires on correct code is worse than no detector.
BROADCAST = re.compile(
    r"(?:vpbroadcast|vbroadcast)[a-z0-9]*\s+[^,]*\(%rip\)|movdq[au]\s+[^,]*\(%rip\)"
)
PCMPEQ = re.compile(r"\bv?pcmpeq[bwdq]\b")
CALLISH = re.compile(r"^\s*(call|callq)\b")


def blocks(lines):
    """label -> (start_index, end_index) of its instruction range."""
    idx = {}
    order = []
    for i, ln in enumerate(lines):
        m = LABEL.match(ln)
        if m:
            idx[m.group(1)] = i
            order.append((m.group(1), i))
    ends = {}
    for k, (name, i) in enumerate(order):
        ends[name] = order[k + 1][1] if k + 1 < len(order) else len(lines)
    return idx, ends


def current_symbol(order_syms, i):
    lo, hi = 0, len(order_syms) - 1
    best = "<unknown>"
    for name, pos in order_syms:
        if pos <= i:
            best = name
        else:
            break
    return best


def store_width(ln):
    """Width in bytes of a store instruction, from its register operand."""
    m = STORE.match(ln)
    if not m:
        return 0
    reg = m.group(1)
    if "ymm" in reg:
        return 32
    if "xmm" in reg:
        # movq %xmm -> 8 bytes; movd -> 4; otherwise a full 16
        mn = ln.strip().split()[0]
        if mn.endswith("q"):
            return 8
        if mn.endswith("d"):
            return 4
        return 16
    if reg.startswith("%r"):
        return 8
    if reg.startswith("%e"):
        return 4
    return 1


def main():
    args = sys.argv[1:]
    sym_filter = None
    if "--sym" in args:
        k = args.index("--sym")
        sym_filter = re.compile(args[k + 1])
        del args[k : k + 2]
    min_trip = 0
    if "--min-trip" in args:
        k = args.index("--min-trip")
        min_trip = int(args[k + 1])
        del args[k : k + 2]
    if args:
        path = args[0]
    else:
        import glob, os

        c = glob.glob("target/release/deps/rusty_zstd-*.s")
        if not c:
            sys.exit("no asm; run: cargo rustc --release -p rusty_zstd -- --emit asm")
        path = max(c, key=os.path.getmtime)

    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    idx, ends = blocks(lines)
    order_syms = [
        (m.group(1), i)
        for i, ln in enumerate(lines)
        if (m := LABEL.match(ln)) and not m.group(1).startswith((".L", "anon."))
    ]

    # An innermost loop: a block whose last instruction jumps BACK to its own
    # label (or to a label at/above its start with no intervening label target).
    findings = []
    for name, start in idx.items():
        if not name.startswith(".L"):
            continue
        end = ends.get(name, start)
        body = lines[start + 1 : end]
        if not body:
            continue
        # does the block jump back to itself?
        backedge = False
        for ln in body:
            m = JMP_ANY.match(ln)
            if m and m.group(2) == name:
                backedge = True
        if not backedge:
            continue
        instrs = [
            ln
            for ln in body
            if ln.strip() and not DIRECTIVE.match(ln) and not LABEL.match(ln)
        ]
        if any(CALLISH.match(ln) for ln in instrs):
            continue  # not a tight kernel loop
        n = len(instrs)
        if n < 3:
            continue
        sym = current_symbol(order_syms, start)
        if sym_filter and not sym_filter.search(sym):
            continue
        has_ymm = any(YMM.search(ln) for ln in instrs)
        has_xmm = any(XMM.search(ln) for ln in instrs)
        widest = 32 if has_ymm else (16 if has_xmm else 8)
        stores = [(ln, store_width(ln)) for ln in instrs if store_width(ln)]
        bytes_out = sum(w for _, w in stores)
        if bytes_out < min_trip:
            continue
        flags = []
        if stores:
            mx = max(w for _, w in stores)
            if mx < widest:
                flags.append(f"HALF-STORE(store {mx}B < reg {widest}B)")
        inv = [ln.strip() for ln in instrs if BROADCAST.search(ln)]
        if inv:
            flags.append(f"INVARIANT({len(inv)} broadcast in body)")
        ncmp = sum(1 for ln in instrs if PCMPEQ.search(ln))
        if ncmp >= 3:
            flags.append(f"SELECT({ncmp} pcmpeq)")
        if not flags:
            continue
        findings.append((sym, name, n, bytes_out, flags))

    findings.sort(key=lambda f: (-len(f[4]), -f[2]))
    print(f"asm: {path}")
    print(f"innermost loops flagged: {len(findings)}\n")
    print(f"{'instrs':>6} {'outB':>6}  symbol / block")
    for sym, blk, n, out, flags in findings:
        short = sym[-60:] if len(sym) > 60 else sym
        print(f"{n:>6} {out:>6}  {short}  [{blk}]")
        for f in flags:
            print(f"{'':>13}  -> {f}")


if __name__ == "__main__":
    main()
