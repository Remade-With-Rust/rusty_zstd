#!/bin/bash
# Per-symbol instruction count from the emitted release asm.
#
# DETERMINISTIC: same toolchain + same source = same number on any machine
# under any load. That is the whole point -- a 2-instruction change is a
# verdict here, where on a drifting box no paired A/B could resolve it.
#
# CAVEATS THAT TRAVEL WITH THE NUMBER:
#  * It cannot price work moved BETWEEN branches. Hoisting a subexpression two
#    sibling `if` arms shared measured +33 instructions while executing strictly
#    less. Ask whether the change is straight-line before trusting the sign.
#  * It systematically favours outlining and branch-merging, so never let it
#    decide `#[inline(never)]`.
#  * It scores every FAST PATH as a loss: the new arm is new code and the old
#    arm has to stay. Price a fast path in CALLS avoided instead.
#
# usage: tools/icount.sh > before.txt ; ... ; tools/icount.sh > after.txt
#        diff <(cut -d' ' -f2- before.txt) ... or tools/icount.sh --diff before.txt
set -u
cd "$(dirname "$0")/.." || exit 1
S=$(ls -t target/release/deps/rusty_zstd-*.s 2>/dev/null | head -1)
[ -z "$S" ] && { echo "no asm; run: cargo rustc --release -p rusty_zstd -- --emit asm" >&2; exit 1; }

dump() {
  awk '
    /^[A-Za-z_$][A-Za-z0-9_$.@]*:/ { sym=substr($0,1,index($0,":")-1); next }
    /^\.?L[A-Za-z0-9_$.]*:/ { next }
    /^[[:space:]]*\./ { next }
    /^[[:space:]]*$/ { next }
    /^[[:space:]]*#/ { next }
    sym != "" { c[sym]++ }
    END { for (s in c) printf "%8d %s\n", c[s], s }
  ' "$S" | sort -rn
}

if [ "${1:-}" = "--diff" ]; then
  BEFORE="$2"
  dump > /tmp/icount_after.$$
  echo "delta  before   after  symbol"
  join -j 2 -o 0,1.1,2.1 <(sort -k2 "$BEFORE") <(sort -k2 /tmp/icount_after.$$) 2>/dev/null \
    | awk '{ d=$3-$2; if (d!=0) printf "%+6d %7d %7d  %s\n", d, $2, $3, $1 }' \
    | sort -n
  rm -f /tmp/icount_after.$$
else
  dump
fi
