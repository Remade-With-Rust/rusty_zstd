#!/usr/bin/env python3
"""Rank the GUARD BRANCHES in a release build: conditional jumps that exist
only to reach a panic.

WHY THIS AND NOT AN INSTRUCTION COUNT
-------------------------------------
A static instruction count is the right primary counter for a straight-line
change, but it is a blunt instrument for proving an index in range: it moves
when inlining moves, and it mixes the guard you removed in with every other
change LLVM made. The guard-branch count isolates exactly that class, and it
does not drift when an inlining boundary shifts.

THE DETECTOR HAS NO FREE PARAMETER, DELIBERATELY
------------------------------------------------
The obvious rule -- "does a panic symbol appear within N lines of the branch
target" -- is a knob, and a knob has to be swept before it can be quoted. Swept
on a real codec it produced 95/115/119/120/125/128/132/143 for budgets 4..64:
monotonic, no plateau, i.e. it was measuring the knob and not the program. Panic
blocks share tails and sit next to one another, so a window either stops short
of the shared `call` or falls through into an unrelated neighbour.

So the rule here is structural: walk the branch target to its FIRST control
transfer (following unconditional jumps), and ask whether that transfer is a
panic call. Nothing to tune, and it cannot drift with block layout.

NAMING THE LINE
---------------
Debug line tables attribute an inlined bounds check to `core/src/slice/index.rs`
-- the check, not the caller -- so every guard in the codec reads as the same
useless line. But every panic call passes a `&core::panic::Location`, which
rustc emits as an `anon.*` rodata object holding {&str file, u32 line, u32 col}.
Reading that names the line of OUR code that failed to prove its index. It needs
no debug info and no source edit, so it cannot perturb what it measures.

usage: python tools/panic_census.py [asm.s] [--all] [--sym REGEX]
"""
import re
import sys
from collections import defaultdict

PANIC_RE = re.compile(
    r"panic_bounds_check|panic_const_(?:div|rem)_by_zero|panic_fmt|"
    r"panic_misaligned|slice_(?:start|end)_index|panic_out_of_range|"
    r"unwrap_failed|panic_no_value|panic_cannot_unwind"
)
COND_JMP = re.compile(r"^\s*(j(?!mp\b)[a-z]{1,3})\s+(\.?L[A-Za-z0-9_$.]+)\s*$")
UNCOND_JMP = re.compile(r"^\s*jmp\s+(\.?L[A-Za-z0-9_$.]+)\s*$")
CALL = re.compile(r"^\s*(?:call|callq|jmp)\s+\*?([A-Za-z_$.][\w$.@]*)")
LABEL = re.compile(r"^(\.?L[A-Za-z0-9_$.]+|[A-Za-z_$][\w$.@]*):")
LEA_ANON = re.compile(r"lea[ql]?\s+(anon\.[0-9a-f]+(?:\.\d+)?)\(%rip\)")
DIRECTIVE = re.compile(r"^\s*\.")


def parse(path):
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    # label -> index of its first instruction line
    label_at = {}
    for i, ln in enumerate(lines):
        m = LABEL.match(ln)
        if m:
            label_at[m.group(1)] = i
    return lines, label_at


def _unescape(sv):
    """Decode a gas .asciz/.ascii operand body into bytes."""
    out = bytearray()
    i = 0
    while i < len(sv):
        c = sv[i]
        if c != "\\":
            out.append(ord(c) & 0xFF)
            i += 1
            continue
        i += 1
        if i >= len(sv):
            break
        d = sv[i]
        if d.isdigit():
            j = i
            while j < len(sv) and j < i + 3 and sv[j].isdigit():
                j += 1
            out.append(int(sv[i:j], 8) & 0xFF)
            i = j
        else:
            out.append({"n": 10, "t": 9, "r": 13, "0": 0}.get(d, ord(d)))
            i += 1
    return bytes(out)


def read_anon_locations(lines):
    """anon.N -> (file_symbol, line).

    rustc emits a `core::panic::Location` as `.quad <file str symbol>` followed
    by a packed payload: u64 string length, u32 line, u32 col. On this target
    the payload is a single `.asciz` with octal escapes rather than `.long`
    directives, so it has to be decoded rather than pattern-matched.
    """
    out = {}
    n = len(lines)
    for i in range(n):
        m = re.match(r"^(anon\.[0-9a-f]+(?:\.\d+)?):", lines[i])
        if not m:
            continue
        name = m.group(1)
        quad = None
        payload = None
        for j in range(i + 1, min(i + 8, n)):
            s = lines[j].strip()
            if re.match(r"^anon\.", s) or re.match(r"^\.section", s):
                break
            q = re.match(r"\.quad\s+([A-Za-z_$.][\w$.@]*)", s)
            if q and quad is None:
                quad = q.group(1)
                continue
            a = re.match(r'\.(?:asciz|ascii)\s+"(.*)"\s*$', s)
            if a and quad is not None:
                payload = _unescape(a.group(1))
                break
        if quad is not None and payload is not None and len(payload) >= 12:
            line_no = int.from_bytes(payload[8:12], "little")
            out[name] = (quad, line_no)
    return out


def read_str_symbols(lines):
    """file-str symbol -> the path it holds."""
    out = {}
    for i, ln in enumerate(lines):
        m = LABEL.match(ln)
        if not m:
            continue
        name = m.group(1)
        for j in range(i + 1, min(i + 4, len(lines))):
            s = lines[j].strip()
            a = re.match(r'\.ascii\s+"(.*)"$', s) or re.match(r'\.asciz\s+"(.*)"$', s)
            if a:
                v = a.group(1)
                if "/" in v or "\\" in v or v.endswith(".rs"):
                    out[name] = v.replace("\\\\", "/")
                break
            if not DIRECTIVE.match(lines[j]):
                break
    return out


def first_transfer(lines, label_at, label, seen=None):
    """Walk from `label` to the first control transfer, following unconditional
    jumps. Returns (kind, operand) where kind is 'call' | 'cond' | 'end'."""
    if seen is None:
        seen = set()
    if label in seen or label not in label_at:
        return ("end", None)
    seen.add(label)
    i = label_at[label] + 1
    while i < len(lines):
        ln = lines[i]
        if LABEL.match(ln):
            # fell through into the next block
            nxt = LABEL.match(ln).group(1)
            return first_transfer(lines, label_at, nxt, seen)
        s = ln.strip()
        if not s or DIRECTIVE.match(ln):
            i += 1
            continue
        u = UNCOND_JMP.match(ln)
        if u:
            return first_transfer(lines, label_at, u.group(1), seen)
        c = CALL.match(ln)
        if c:
            return ("call", c.group(1))
        if COND_JMP.match(ln):
            return ("cond", COND_JMP.match(ln).group(2))
        if re.match(r"^\s*(ret|ud2|int3)", ln):
            return ("end", s.split()[0])
        i += 1
    return ("end", None)


def anon_in_block(lines, label_at, label, seen=None):
    """Find the Location operand reachable from `label`."""
    if seen is None:
        seen = set()
    if label in seen or label not in label_at:
        return None
    seen.add(label)
    i = label_at[label] + 1
    while i < len(lines):
        ln = lines[i]
        if LABEL.match(ln):
            return anon_in_block(lines, label_at, LABEL.match(ln).group(1), seen)
        m = LEA_ANON.search(ln)
        if m:
            return m.group(1)
        u = UNCOND_JMP.match(ln)
        if u:
            return anon_in_block(lines, label_at, u.group(1), seen)
        if CALL.match(ln) or re.match(r"^\s*(ret|ud2)", ln):
            return None
        i += 1
    return None


def current_symbol(lines, upto):
    """Nearest preceding non-.L label -- the function this block belongs to."""
    for i in range(upto, -1, -1):
        m = LABEL.match(lines[i])
        if m and not m.group(1).startswith(".L") and not m.group(1).startswith("anon."):
            return m.group(1)
    return "<unknown>"


def main():
    args = [a for a in sys.argv[1:]]
    show_all = "--all" in args
    args = [a for a in args if a != "--all"]
    sym_filter = None
    if "--sym" in args:
        k = args.index("--sym")
        sym_filter = re.compile(args[k + 1])
        del args[k : k + 2]
    if args:
        path = args[0]
    else:
        import glob, os

        cands = glob.glob("target/release/deps/rusty_zstd-*.s")
        if not cands:
            sys.exit("no asm found; run: cargo rustc --release -p rusty_zstd -- --emit asm")
        path = max(cands, key=os.path.getmtime)

    lines, label_at = parse(path)
    anon = read_anon_locations(lines)
    strs = read_str_symbols(lines)

    per_sym = defaultdict(int)
    per_site = defaultdict(int)
    total = 0
    for i, ln in enumerate(lines):
        m = COND_JMP.match(ln)
        if not m:
            continue
        kind, op = first_transfer(lines, label_at, m.group(2))
        if kind != "call" or not op or not PANIC_RE.search(op):
            continue
        total += 1
        sym = current_symbol(lines, i)
        if sym_filter and not sym_filter.search(sym):
            continue
        per_sym[sym] += 1
        a = anon_in_block(lines, label_at, m.group(2))
        if a and a in anon:
            fsym, line_no = anon[a]
            f = strs.get(fsym, fsym)
            per_site[(f, line_no)] += 1
        else:
            per_site[("<unresolved>", 0)] += 1

    print(f"asm: {path}")
    print(f"guard branches (conditional jumps whose target's first transfer is a panic call): {total}\n")
    print(f"{'guards':>7}  symbol")
    n = len(per_sym) if show_all else 25
    for sym, c in sorted(per_sym.items(), key=lambda kv: -kv[1])[:n]:
        print(f"{c:>7}  {sym}")
    print(f"\n{'guards':>7}  source site (the line that failed to prove its index)")
    for (f, l), c in sorted(per_site.items(), key=lambda kv: -kv[1])[: (None if show_all else 30)]:
        short = f.split("/")[-1] if f != "<unresolved>" else f
        print(f"{c:>7}  {short}:{l}")


if __name__ == "__main__":
    main()
