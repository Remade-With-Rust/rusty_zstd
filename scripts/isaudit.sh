#!/bin/bash
# ISA twin audit -- the check no correctness test can make.
#
# A `#[target_feature]` twin is byte-identical to its baseline sibling by
# construction, so the suite cannot see whether the twin does anything at all.
# This reads the emitted asm and answers that directly: a twin is working only
# if it carries the ISA ops that its baseline sibling carries in `%cl`.
#
# Columns:
#   instr  total instructions in the symbol
#   cl     variable-count shifts (`shl/shr/sar/rol/ror ..., %cl`) -- 3 uops on
#          Intel, and exactly what BMI2 `shlx`/`shrx` exist to replace
#   bmi2   BMI2/ABM ops present (shlx, shrx, bzhi, pdep, pext, mulx, ...)
#   vex    VEX-encoded (AVX) instructions
#
# Two failure shapes to look for:
#   * a `_bmi2`/`_avx2` symbol with bmi2==0 while its baseline sibling has
#     cl>0 -- a THUNK twin: dispatches correctly, does nothing (four have
#     shipped in this crate)
#   * a baseline symbol with a low instr/cl ratio and no twin -- an
#     un-twinned hot shift carrier. Below ~40 instructions per converted op a
#     twin tends to pay; above it the duplicated body costs more I-cache than
#     the shifts are worth. `find_dfast`'s retired twin was 70/op.
#
# usage: scripts/isaudit.sh [regex-filter]
#   prerequisite: cargo rustc --release -p rusty_zstd -- --emit asm
set -u
cd "$(dirname "$0")/.." || exit 1

S=$(ls -t target/release/deps/rusty_zstd-*.s 2>/dev/null | head -1)
if [ -z "$S" ]; then
  echo "no .s found -- run: cargo rustc --release -p rusty_zstd -- --emit asm" >&2
  exit 1
fi

FILTER="${1:-.}"

awk '
  /^_[ZR].*:$/ && !/^[[:space:]]/ {
      if (name != "") print count "\t" cl "\t" bmi2 "\t" vex "\t" name
      name = substr($0, 1, length($0)-1); count = 0; cl = 0; bmi2 = 0; vex = 0
      next
  }
  name != "" && /^[[:space:]]+[a-z]/ {
      count++
      if ($0 ~ /(shl|shr|sar|rol|ror)[bwlq]?[[:space:]]+%cl/) cl++
      if ($0 ~ /[[:space:]](shlx|shrx|sarx|bzhi|pdep|pext|mulx|andn|blsr|blsi|blsmsk|bextr|tzcnt|lzcnt)/) bmi2++
      if ($0 ~ /[[:space:]]v[a-z]/) vex++
  }
  END { if (name != "") print count "\t" cl "\t" bmi2 "\t" vex "\t" name }
' "$S" |
sort -rn |
python3 -c '
import sys, re

def demangle(s):
    """Readable name from either Rust mangling.

    Parse LENGTH PREFIXES rather than stripping digits: names here are full of
    meaningful digits (decode_4x_x1, find_fast_impl_bmi2, decode_sequences_avx2)
    and a blanket digit strip renders them indistinguishable -- that bug made a
    filter for `decode_4x_x1` silently match nothing and report a symbol as
    absent when it was right there.

    Both manglings are handled because this tree emits both depending on how
    the build was invoked, and an audit that silently returns zero rows for the
    wrong one reads exactly like "clean".
    """
    if s.startswith("_ZN"):                      # legacy: _ZN <len><seg>... 17h<hash> E
        i, segs = 3, []
        while i < len(s) and s[i].isdigit():
            j = i
            while j < len(s) and s[j].isdigit():
                j += 1
            n = int(s[i:j])
            seg = s[j:j + n]
            if not re.fullmatch(r"h[0-9a-f]{16}", seg):
                segs.append(seg)
            i = j + n
        return "::".join(segs) if segs else s
    if s.startswith("_R"):                       # v0: interleaved tags + <len><ident>
        segs, i = [], 2
        while i < len(s):
            if s[i].isdigit():
                j = i
                while j < len(s) and s[j].isdigit():
                    j += 1
                n = int(s[i:j])
                # v0 allows a `_` separator before idents that start with a digit
                k = j + 1 if j < len(s) and s[j] == "_" else j
                seg = s[k:k + n]
                if len(seg) == n and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", seg or "x"):
                    segs.append(seg)
                    i = k + n
                    continue
            i += 1
        # Drop crate-disambiguator noise ("Cs<hash>") and keep the tail, which
        # is where the function name lives.
        segs = [x for x in segs
                if not re.fullmatch(r"[A-Za-z]?s?[0-9A-Za-z]{10,}", x)
                and not re.fullmatch(r"[A-Za-z]{0,2}_?\d+[A-Za-z]{0,2}", x)
                and x != "rusty_zstd"]
        return "::".join(segs[-3:]) if segs else s
    return s

flt = re.compile(sys.argv[1]) if len(sys.argv) > 1 else re.compile(".")
print("%6s %5s %5s %5s  symbol" % ("instr", "cl", "bmi2", "vex"))
for line in sys.stdin:
    parts = line.rstrip("\n").split("\t")
    if len(parts) != 5:
        continue
    c, cl, b, v, name = parts
    sym = demangle(name)
    if flt.search(sym):
        print(f"{int(c):6d} {int(cl):5d} {int(b):5d} {int(v):5d}  {sym}")
' "$FILTER"
