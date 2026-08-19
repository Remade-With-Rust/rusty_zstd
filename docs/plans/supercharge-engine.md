# Supercharge the Engine

**Thesis.** The Great Gate campaign is working on the axis it was built for. It is
not the instrument that closes the speed gap, because the gap is not *at* the
gates — it is inside the code the gates route to. This plan names that code,
prices it, and records what may and may not be used to measure it.

Status: **evidence gathered, nothing built.** Every number below is measured and
reproducible. Every proposal below is unverified until it is measured.

---

## 1. The gap

From the L3 baseline board in `gg-Addendum.md` (`C/us c` = C's compress speed ÷
ours; above 1.0 means C is faster):

| | mean `C/us c` | range |
| --- | ---: | --- |
| **14 search-heavy corpora** | **2.36** | 1.93 (`reymont`) → 2.95 (`ooffice`) |
| 4 degenerate corpora | 0.59 | `zeros` 0.26, `text` 0.50, `versions` 0.68, `incomp` 0.90 |

Mean board ratio is **1.012** and we are *smaller* than C on 3 corpora
(`x-ray` 0.983). **We are not buying the time back with compression ratio. We are
spending it.**

## 2. Why gating cannot close it

Three independent readings of the same board, in increasing order of force:

1. **The multiplier is nearly constant.** 1.93 → 2.95 across DNA, satellite
   imagery, English prose, XML, executables and databases — content with almost
   nothing in common. A wrong gate decision produces *scatter*, because it fires
   on some content and not others. A flat multiplier across everything is the
   signature of a **fixed per-position cost**.
2. **We win precisely where the loop does not run.** The four corpora we beat C
   on are RLE (`zeros`, 3.8× faster), degenerate repetition (`text`), repcode-
   driven (`versions`) and early-raw-skip (`incomp`). **We beat C whenever we
   avoid work and lose whenever we do work.**
3. **Decompression is 2.04× slower too** — and the decoder has almost no policy
   decisions to gate (`dickens` 3.11, `mr` 3.13, `x-ray` 2.42). The same gap
   exists where there is nothing to gate. This one is decisive on its own.

A gate can only remove work it can *prove* is unnecessary: remove 10% of
positions, get 10%. Closing 2.36× requires making the position cheaper.

**Consistent with this: none of the campaign's real speed wins were gates.**
`run_jobs` rewrite −19.84%; Bt chain-write removal to −28.8%; prefix copy bound
to −21.9%; Gate 13 literal width −14.62% instructions. All four are dead-work
removal or instruction-count reduction.

## 3. Where the time actually goes

Measured with the `profile` feature, 8 MiB board (`hotspot.rs`). Stage timers are
distorted by their own instrumentation, so they are read **only as shares, only
to rank**. The counters beside them are exact.

| level | matchfind | entropy | huff | fse | seqcode |
| --- | ---: | ---: | ---: | ---: | ---: |
| **L3** (DFast) | **75.3%** | 21.3% | 7.9% | 6.3% | 5.5% |
| **L1** (Fast) | **65.2%** | 31.9% | **21.5%** | 4.7% | 4.0% |

**Match-find is three quarters of encode time at L3.** Nothing else is in the
same class. At L1 huffman rises to 21.5% and is a genuine secondary target — on
literal-heavy content it dominates outright (`sao` at L1: huff **33.2%**,
literals **95.9%** of input).

## 4. The signal inside match-find

`probes/B` = hash probes per input byte. `hit%` = probes that found a usable
candidate. Both exact and undistorted.

| corpus | `C/us c` | probes/B | **hit%** | find% |
| --- | ---: | ---: | ---: | ---: |
| ooffice | 2.95 | 1.07 | **5.4%** | 76.9% |
| sao | 2.79 | 1.84 | **3.0%** | 78.4% |
| smallmsg-8m | 2.69 | 0.72 | **8.8%** | 81.1% |
| x-ray | 2.66 | 1.69 | **5.4%** | 77.4% |
| osdb | 2.64 | 0.41 | 13.1% | 70.9% |
| jsonlog-16m | 2.57 | 0.30 | 17.5% | 76.0% |
| mozilla | 2.36 | 0.49 | 4.7% | 78.2% |
| mr | 2.34 | 0.51 | 18.7% | 74.2% |
| samba | 2.12 | 0.22 | 19.3% | 73.5% |
| nci | 2.11 | 0.04 | **57.8%** | 65.8% |
| dickens | 1.98 | 0.41 | **27.2%** | 74.3% |
| webster | 1.95 | 0.32 | **26.7%** | 76.8% |

**r(hit%, `C/us`) = −0.693**, **r(probes/B, `C/us`) = +0.675**, n = 12.
At the tails: **hit% < 10% → mean `C/us` 2.69; hit% > 18% → mean 2.10.**

**The corpora where our probes miss are the corpora where we are furthest behind
C.** On `sao`, `ooffice` and `x-ray`, **95%+ of every probe finds nothing** — and
a missing probe still costs the hash, the (uncached) candidate load, and the
compare.

`probes/B` is *below 1.0* on 9 of 12, so **we are not over-searching.** We do not
issue too many probes. Each probe is too expensive, and too many are wasted.
Those are two different defects; both live in the same function.

---

## 5. Engine targets inside match-find, ranked

### T1 — DFast never uses the rejection tag — **SHIPPED, default ON**

**Result: byte-identical on 18/18 at L3 and 72/72 across the board, removing
2,938,472 candidate loads per board pass for zero added work.**

The tag rejects **29.8%** of non-empty short slots, and the rejection lands where
the gap is: `mozilla` 67.6%, `ooffice` 60.5%, `osdb` 57.7%, `x-ray` 46.1%,
`samba` 43.0%, `sao` 41.6% — against `dickens` 5.8%, `webster` 9.9%, `nci` 2.9%,
the three we were already closest to C on.

**Two things had to be got right, and the first attempt got neither.**

1. *The separate tag array does not pay.* Allocating `tags` for DFast worked and
   was byte-identical, but it added **one tag store per position** to avoid ~0.3
   candidate loads per position — a second cache line touched every time round
   the loop, for ~3.3 stores per avoided load. The clock agreed (+3.76%).
   **Packing the tag into the top 8 bits of the slot the finder already loads
   makes it free**: same word, same load, same store, and no array at all.
   Sound only while `pos + 1` fits in 24 bits, so `enable_packed_tags` proves
   that per frame against the real buffer length rather than assuming it.
2. *Every writer of that table must share the representation.*
   `fill_hash_after_match` runs after **every match** on the DFast path and wrote
   the slot unpacked; a packed reader then decoded the tag bits as part of the
   position. **Output moved on 12 of 18 corpora** until it was routed through the
   same writer. This is the third time this codebase has been bitten by one
   writer skipping the tag — `store_fast`'s own comment (190ad8b) and
   `prime_tables` were the first two. `prime_tables` needed the same fix here.

*The clock still cannot confirm it* — a 9-round paired test with a null arm put
every corpus inside the floor (`mozilla` −12.12% against a 16.68% floor). It
ships on the deterministic case: **strictly fewer loads, strictly less memory,
byte-identical**, which is the same standard that shipped the prefix bound and
the Bt chain-write removal.

**OPEN:** the 24-bit guard disables packing for frames at or above 16 MiB, so
large single frames get no filter. A window-relative or 25-bit encoding would
extend the coverage.

### T1 — original analysis
**The strongest lead on the board, and it is already built.**

`find_fast_impl` (L1/L2) is generic over `const PACKED: bool` and uses the packed
tag in **28 places**: `load_fast::<true>` compares a tag byte held *in the table
entry* and rejects a bad candidate **without ever loading the candidate's bytes**
— i.e. without the random-access cache miss that dominates a missing probe.

`find_dfast_impl` (L3/L4) calls `hash4_tag::<false>` and `store_fast::<false>`.
**PACKED is false. DFast never uses the mechanism at all.**

Priced by `tagprice.rs` — what the tag already saves at L1, where it *is* used:

| corpus | probes | candidate loads avoided | **saved%** | false rejects |
| --- | ---: | ---: | ---: | ---: |
| sao | 3,576,126 | 1,375,142 | **38.5%** | **0 (0.00%)** |
| mozilla | 2,031,456 | 777,509 | **38.3%** | **0** |
| x-ray | 135,250 | 49,477 | **36.6%** | **0** |
| ooffice | 3,616,409 | 1,257,485 | **34.8%** | **0** |
| osdb | 3,181,411 | 1,091,060 | **34.3%** | **0** |
| smallmsg-8m | 3,322,514 | 1,100,914 | **33.1%** | **0** |
| samba | 1,879,602 | 123,600 | 6.6% | 0 |
| dickens | 4,969,521 | 8,467 | 0.2% | 0 |
| webster | 3,329,935 | 7,780 | 0.2% | 0 |
| nci | 615,104 | 336 | 0.1% | 0 |

Two things make this the top target:

* **The tag eliminates 33–38% of candidate loads on exactly the corpora where we
  are worst against C** (`sao` 2.79, `ooffice` 2.95, `x-ray` 2.66, `smallmsg`
  2.69, `osdb` 2.64, `mozilla` 2.36) — and ~0% on the corpora where we are
  already closest (`dickens` 1.98, `webster` 1.95, `nci` 2.11). The saving lands
  where the gap is, corpus for corpus.
* **Zero false rejects on every corpus measured.** The tag costs no ratio at all
  in the data we have. It is not a size-for-speed trade.

*Unverified and must be measured:* whether DFast's two-table layout leaves tag
bits free; whether the extra store costs more than the avoided loads; and whether
the L1 saving rate transfers to DFast's different hash and step pattern. **The
32% is what the tag buys on the Fast ladder, not a promise for DFast.**

### T2 — Probe cost in the inner loop — **TRANCHE 1 SHIPPED**

Measured from the emitted asm (`--emit asm`, parsed per symbol):

| function | copies | instrs | spill slots | **panic sites** |
| --- | ---: | ---: | ---: | ---: |
| `find_fast_impl` (L1/L2) | **34** | 39,424 | 31 | **0** |
| `find_dfast_impl` (L3/L4) | 1 | 1,713 | 60 | **13 → 0** |
| `find_greedy` (L5) | 1 | 675 | — | 5 |
| `find_lazy` (L6–L12) | 1 | 807 | — | 3 |
| `find_bt_lazy` (L13–L15) | 1 | 677 | — | 2 |
| **`bt_find_best`** (Bt core) | **23** | **9,058** | — | **132** |

**Shipped:** the Fast finder carries 0 panic sites because brick 50 proved its
index invariant and moved to `get_unchecked`. DFast never got that treatment, so
it paid a compare-and-branch on **every** table access — four per position (short
and long, read and write). The invariant is the *same* one, and this file already
depends on it: every index is `hash4`/`hash8`/`hash4_tag` shifted down to
`tables.hash_log` bits, into tables allocated `1 << tables.hash_log`. Verified at
all six finder bindings before touching it. **13 → 0, byte-identical on 72/72.**

**NOT done, and deliberately — `bt_find_best`'s 132 panic sites are NOT safe to
remove as written.** The tree indexes `chain[(m & bt_mask) << 1]` and `+1`, so
the maximum index is `2^(bt_log+1) - 1`. But `bt_log` comes from the **const
generic `CLOG`**, not from the table's real length — the exact mistake the hash
tables document and avoid by binding `tables.hash_log` rather than
`params.hash_log`. Worse, `btlog` floors at `.max(1)`, so `bt_mask >= 1` and the
tree needs `chain.len() >= 4`, while the only guard present is
`chain.len() < 2`. With `params.chain_log = 1` (reachable through the advanced
API) the tree addresses index 3 into a 2-entry table.

**That bounds check is currently load-bearing: it converts a latent
out-of-bounds into a panic.** Removing it would convert it into UB. The correct
order is to derive `bt_mask` from `tables.chain.len()` — making the invariant
provable *and* closing the edge — and only then take the checks out. That is a
behaviour-affecting change on the Bt ladder and needs its own verification pass.

**Register pressure remains open.** `find_dfast_impl` holds 60 distinct spill
slots and reloads one of them 20 times against a single store — a loop-invariant
value LLVM refuses to keep in a register. And `find_fast_impl` has **34
monomorphisations totalling 39,424 instructions**. Note that dead specialisations
cost binary size and compile time, *not* execution — do not price them as speed
without measuring which ones serve calls (GATE 12 found a Bt specialisation
serving 0%).

### T2 — original note
Already diagnosed in the code, never resolved. From `find_fast_impl`:

> *"a function this large is why LLVM spills the src base in the prologue and
> rematerializes it on every probe even with six callee-saved registers idle"*

Brick 59 shrank the function by making `pipe_enabled()` a const. The observation
that it is still too large for good register allocation stands. This is the
`probes/B < 1.0` half of §4: we are not over-probing, so what remains is per-probe
cost. Instrument: **instructions per position from emitted asm** — the Gate 13
method, the only one that has beaten the noise floor.

### T3 — Huffman at L1, 21.5% of encode
Third by share, first by *concentration*: on literal-heavy content it is the
largest single consumer (`sao` 33.2%, `ooffice` 23.1%, `mr` 20.4%). Scope any work
here to literal-dominated input at L1/L2, where match-find has little to do.

### T4 — The decoder, 2.04× and almost ungated
`dickens` 3.11, `mr` 3.13, `x-ray` 2.42. Untouched by this campaign,
structurally simpler than the encoder, and the cleanest proof that the gap is
implementation rather than policy. Probably the best effort-to-reward on the board.

---

## 6. Measured and DECLINED — do not re-open

Recorded so nobody spends another session rediscovering these.

**Per-block allocation work is finished, and it was never a speed lever.** The
premise behind three Gate 6 cells — "≥128 KiB leaves the low-fragmentation heap"
— is **false as a price**, measured by `allocost.rs`:

| buffer size | fresh ns/op | reused ns/op | fresh − reused |
| ---: | ---: | ---: | ---: |
| 128 KiB | 3,251 | 2,836 | +415 ns |
| 134 KiB | 3,117 | 3,372 | **−256 ns** |
| 512 KiB | 13,530 | 13,682 | **−152 ns** |
| **1 MiB** | **392,206** | **33,277** | **+358,929 ns** |

No cliff at 128 KiB — the sign even flips. The cliff is at **1 MiB (+1078%)**, the
OS zero-filling pages the heap no longer recycles.

* `huffman::encode_literals_section` (65 × ~138 KB/frame): **~63 µs against a
  96 ms frame = 0.07%. DECLINED.**
* Gate 6 @ L1 (`lits`, 64/frame): ~0 in time. A 9-round paired test with its own
  null arm put **every corpus inside the floor**. Kept as hygiene, not speed.
* Gate 6 @ L19 (`opt_ops`, **4 MiB per block**) is the **only** allocation fix
  that crosses the real cliff, by 4×. Plausibly ~11 ms/frame. **Still owed a
  timing pass** — the one open item in this section.

## 7. Method — what is admissible

Learned the hard way; violating these produced two false findings and one
dispatch that had to be thrown away.

1. **The per-frame clock cannot decide anything at this scale.** A null arm
   measuring a setting against **itself** reads up to **±24.15%** (`jsonlog`
   +24.15%, `x-ray` +13.82%). Any effect smaller than that is unmeasurable by
   stopwatch.
2. **Always run the null arm** — not the A/B, the A/A. Six of eight corpora
   sign-flipped across three ABBA rounds before the null arm explained why.
3. **Admissible instruments**, in order: instructions counted from emitted asm
   (Gate 13, −14.62%); deterministic work counters (`probes/B`, `hit%`,
   `take_tag_rejects`, `take_prime_iters`); allocator counters via a counting
   `GlobalAlloc`; byte-identity fingerprints across the 72-cell board.
4. **A real content split can still be fitting to a defect.** Gate 6's
   exact-vs-blanket dispatch measured a genuine, reproducible split — and it
   dissolved once the underlying dead-byte memcpy was fixed.
5. **Size is the guard, not the prize.** Mean ratio 1.012; worst 1.100 (`nci`).
   Any engine change must hold the 72-cell byte-identity board, or justify a
   ratio move on the **worst** corpus — never the mean.

## 8. Order of work

1. ~~**T1** — carry the packed tag to DFast.~~ **DONE, shipped default ON.**
   Next: lift the 24-bit cap so frames >= 16 MiB get the filter too.
2. **T4** — decoder. Simplest structure, 2.04× behind, nothing to gate.
3. **T2** — inner-loop codegen and register pressure. Slowest to verify.
4. **T3** — huffman, scoped to literal-heavy content at L1.
5. **Owed:** the L19 `opt_ops` timing pass from §6.

Harnesses in place: `hotspot.rs` (stage + counter map), `tagprice.rs` (what the
tag buys), `allocost.rs` (allocation pricing), `g6null.rs` (noise floor),
`g6time.rs` (paired timing with a null arm).
