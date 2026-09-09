#!/usr/bin/env bash
# Shipped-CLI vs shipped-zstd, same input, single-threaded both sides.
#
# Why CLI-vs-CLI and not the in-process bench: `rzstd-bench` installs
# `rzstd-alloc` as its global allocator, and that allocator's background thread
# is charged to our process. Pinned to ONE core it contends with the codec
# thread -- `cores_busy` reads ~2.0 against the reference's 1.0, which is a
# work-parity violation and voids the comparison. Our CLI uses the system
# allocator (control measured: cores_busy 0.72-0.91), so CLI-vs-CLI is
# like-for-like.
#
# Discipline: arms ABBA-alternated so drift cancels instead of landing on one
# arm; min-of-N, because the floor is what survives a noisy box; a NULL arm
# (ours against itself) to establish what this box can resolve; and work parity
# asserted per row by decoding BOTH outputs and comparing byte counts.
set -u
Z=${Z:-./third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe}
US=${US:-./target/release/rzstd.exe}
REPS=${REPS:-5}
LEVELS=${LEVELS:-"1 3 9"}
CORPORA=${CORPORA:-"dickens samba webster mozilla x-ray nci xml osdb"}

t() { python -c "import time;print(repr(time.perf_counter()))"; }
mn() { python -c "print(repr(min($1,$2)))"; }

printf "%-9s %2s %9s %10s %10s %7s %9s %7s\n" \
  corpus L src_MiB us_MB/s zstd_MB/s "zstd/us" size_us/c null
for id in $CORPORA; do
  f="corpora/data/silesia/$id"; [ -f "$f" ] || f="corpora/data/generated/$id"
  [ -f "$f" ] || continue
  src=$(stat -c%s "$f")
  for L in $LEVELS; do
    "$US" -"$L" -c "$f" > /tmp/vs_us.zst 2>/dev/null
    "$Z"  -"$L" -T1 -c "$f" > /tmp/vs_c.zst 2>/dev/null
    us_b=$(stat -c%s /tmp/vs_us.zst); c_b=$(stat -c%s /tmp/vs_c.zst)
    d1=$("$US" -d -c /tmp/vs_us.zst 2>/dev/null | wc -c)
    d2=$("$Z"  -d -c /tmp/vs_c.zst  2>/dev/null | wc -c)
    if [ "$d1" != "$src" ] || [ "$d2" != "$src" ]; then
      printf "%-9s %2s  VOID work parity: decoded %s / %s vs src %s\n" "$id" "$L" "$d1" "$d2" "$src"
      continue
    fi
    bu=1e9; bc=1e9; bn=1e9
    for i in $(seq 1 "$REPS"); do
      if [ $((i % 2)) -eq 0 ]; then
        a0=$(t); "$US" -"$L" -c "$f" >/dev/null 2>&1; a1=$(t)
        b0=$(t); "$Z" -"$L" -T1 -c "$f" >/dev/null 2>&1; b1=$(t)
      else
        b0=$(t); "$Z" -"$L" -T1 -c "$f" >/dev/null 2>&1; b1=$(t)
        a0=$(t); "$US" -"$L" -c "$f" >/dev/null 2>&1; a1=$(t)
      fi
      n0=$(t); "$US" -"$L" -c "$f" >/dev/null 2>&1; n1=$(t)
      bu=$(mn "$bu" "$a1-$a0"); bc=$(mn "$bc" "$b1-$b0"); bn=$(mn "$bn" "$n1-$n0")
    done
    python - <<PY
mib=$src/1048576.0; bu=$bu; bc=$bc; bn=$bn
print(f"{'$id':<9} {'$L':>2} {mib:9.1f} {mib/bu:10.1f} {mib/bc:10.1f} "
      f"{bu/bc:7.2f} {$us_b/$c_b:9.4f} {bn/bu:7.3f}")
PY
  done
done
echo
echo "zstd/us = how many times faster the C reference is. size_us/c = our output"
echo "over theirs. null = our arm against itself; a result no further from 1.0"
echo "than the null is not a result. Work parity asserted per row."
