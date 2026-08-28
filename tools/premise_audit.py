#!/usr/bin/env python3
"""Premise audit for rusty_zstd -- find decisions that have outlived their reason.

An `#[inline(always)]` or a `#[target_feature]` twin is a DECISION whose
justification is a claim about a BODY. Bodies move. Seven times this campaign a
decision outlived its claim, and every one was found by accident.

Three instruments, in descending order of how well they have actually worked:

  1. TWIN DENSITY (asm)    -- what does this twin's ISA convert, against what
                              the duplicate costs? Found: write_literals_bmi2,
                              encode_block_bmi2, find_sequences{,_strategy}_bmi2,
                              write_sequences_avx2, decode_4x_avx2.
  2. INLINE CENSUS (src)   -- body lines x call sites. Found: FseCTable::from_norm,
                              read_table, prime_tables, parse_ncount_into.
  3. LOCATIVE CLAIMS (src) -- comments asserting code lives in a body that no
                              longer contains it. NOISY: reports variables as
                              often as premises. Use as a reading list only.

Usage:  python tools/premise_audit.py [target/release/deps/rusty_zstd-<hash>.s]
"""
import re, sys, glob, os

SRC = sorted(glob.glob('crates/rusty_zstd/src/*.rs'))

def asm_path():
    if len(sys.argv) > 1: return sys.argv[1]
    c = glob.glob('target/release/deps/rusty_zstd-*.s')
    return max(c, key=os.path.getmtime) if c else None

def symbols(path):
    cur, buf, out = None, [], {}
    for line in open(path, encoding='utf-8', errors='ignore'):
        m = re.match(r'^([A-Za-z_][A-Za-z0-9_$.]*):\s*$', line)
        if m:
            if cur: out[cur] = buf
            cur = re.sub(r'17h[0-9a-f]*E$', '', m.group(1)).replace('_ZN10rusty_zstd', '')
            buf = []
            continue
        if cur: buf.append(line)
    if cur: out[cur] = buf
    return out

def twin_density(path):
    print('=' * 92)
    print('1. TWIN DENSITY -- what the ISA converts vs what the duplicate costs')
    print('=' * 92)
    syms = symbols(path)
    rows = []
    for k, b in syms.items():
        if not re.search(r'(bmi2|avx2)', k): continue
        n = sum(1 for l in b if re.match(r'^\t[a-z]', l))
        if n < 100: continue
        ops = sum(1 for l in b if re.match(
            r'^\t(shlx|shrx|sarx|bzhi|pext|pdep|lzcnt|tzcnt)', l))
        ymm = sum(1 for l in b if '%ymm' in l)
        math = sum(1 for l in b if re.search(
            r'^\tvp?(add|sub|mul|and|or|xor|cmp|shuf|unpck|blend|max|min|perm|pack|sad|madd)', l))
        vex = sum(1 for l in b if re.match(r'^\tv[a-z]', l))
        rows.append((n / max(ops + math, 1), n, ops, ymm, math, vex, k))
    rows.sort(reverse=True)
    print(f"{'instrs/ISA-op':>13}{'instrs':>8}{'bmi2':>6}{'ymm':>5}{'vmath':>7}{'VEX':>5}  twin")
    for ratio, n, ops, ymm, math, vex, k in rows:
        flag = ''
        if ops + math == 0: flag = '  <== converts NOTHING'
        elif ymm and math == 0: flag = '  <== ymm but no vector math (spill widening)'
        elif ratio > 100: flag = '  <== thin'
        print(f"{ratio:13.0f}{n:8d}{ops:6d}{ymm:5d}{math:7d}{vex:5d}  {k}{flag}")
    print('\n  An avx2 twin should be compared against its bmi2 sibling, not the baseline:')
    print('  the file rule is "enable avx2 where the instruction count DROPS; revert where it GROWS".')

def inline_census():
    print()
    print('=' * 92)
    print('2. INLINE CENSUS -- body lines x call sites (the cost of an inline(always))')
    print('=' * 92)
    src = {os.path.basename(f): open(f, encoding='utf-8').read() for f in SRC}
    alltext = '\n'.join(src.values())
    rows = []
    for fname, txt in src.items():
        lines = txt.split('\n')
        for i, l in enumerate(lines):
            if l.strip() != '#[inline(always)]': continue
            for j in range(i + 1, min(i + 9, len(lines))):
                m = re.match(r'\s*(?:pub(?:\(crate\))? )?(?:unsafe )?fn ([a-z_0-9]+)', lines[j])
                if not m: continue
                name = m.group(1)
                indent = len(lines[j]) - len(lines[j].lstrip())
                end = next((k for k in range(j + 1, len(lines))
                            if lines[k] == ' ' * indent + '}'), None)
                if end is None: break
                body = end - j
                sites = len(re.findall(
                    r'[.\s(]' + re.escape(name) + r'\s*(?:::<[^>]*>)?\s*\(', alltext)) - 1
                if body >= 25 and sites >= 2:
                    rows.append((body * sites, body, sites, fname, name))
                break
    rows.sort(reverse=True)
    print(f"{'cost':>7}{'lines':>7}{'sites':>7}  file::fn")
    for c, b, s, f, n in rows[:15]:
        print(f"{c:7d}{b:7d}{s:7d}  {f}::{n}")
    print('\n  `sites` counts TEST call sites too -- check before acting. A high count with a')
    print('  small body is usually fine; a large body at setup frequency is the win.')

def locative():
    print()
    print('=' * 92)
    print('3. LOCATIVE CLAIMS -- comments asserting code lives in a body (NOISY)')
    print('=' * 92)
    LOC = re.compile(r'live[s]? in here|lives in|carr(?:y|ies|ied)|its own|'
                     r'compiles the whole|stamped into', re.I)
    NAMED = re.compile(r'`([a-z_][a-z_0-9]*)`')
    DEC = re.compile(r'^\s*#\[(inline\(always\)|target_feature\([^)]*\))\]')
    FN = re.compile(r'^\s*(?:pub(?:\(crate\))? )?(?:unsafe )?fn ([a-z_0-9]+)')
    hits = 0
    for path in SRC:
        lines = open(path, encoding='utf-8').read().split('\n')
        for i, l in enumerate(lines):
            if not DEC.match(l): continue
            j, block = i - 1, []
            while j >= 0 and (lines[j].lstrip().startswith('//') or DEC.match(lines[j])):
                if lines[j].lstrip().startswith('//'): block.append(lines[j])
                j -= 1
            comment = '\n'.join(reversed(block))
            if not comment or not LOC.search(comment): continue
            fn = next((FN.match(lines[k]).group(1) for k in range(i + 1, min(i + 10, len(lines)))
                       if FN.match(lines[k])), None)
            if not fn: continue
            k0 = next(k for k in range(i + 1, len(lines)) if FN.match(lines[k]))
            indent = len(lines[k0]) - len(lines[k0].lstrip())
            end = next((k for k in range(k0 + 1, len(lines))
                        if lines[k] == ' ' * indent + '}'), k0)
            body = '\n'.join(lines[k0:end])
            miss = sorted({n for n in NAMED.findall(comment)
                           if len(n) > 4 and n != fn
                           and not re.search(r'\b' + re.escape(n) + r'\b', body)})
            if miss:
                hits += 1
                print(f"  {os.path.basename(path):<18}{i+1:>6}  {fn}")
                print(f"{'':<26}names: {', '.join('`'+m+'`' for m in miss[:6])}")
    print(f"\n  {hits} candidates. Expect false positives -- variables and cross-references")
    print("  read the same as premises. This is a reading list, not a verdict.")

a = asm_path()
if a: twin_density(a)
else: print('no .s found -- build with RUSTFLAGS="--emit asm" for section 1')
inline_census()
locative()
