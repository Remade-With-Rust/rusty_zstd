# fast-trans — applying `rusty-fast-transcendentals` to rusty_zstd

**Verdict: the skill's literal target does not exist here, and its underlying law
is already satisfied. Zero bricks recommended. This document is the evidence, so
the question does not get re-opened from scratch.**

Everything below is a deterministic count read out of the release assembly
(`RUSTFLAGS="--emit asm" cargo build -p rusty_zstd --release`, single CGU). No
clocks — the box is CPU-contended and every number here is a count, an
attribution or a line range.

---

## 1. The literal target: zero sites

The skill replaces a per-element `exp`/`ln`/`tanh`/`powf` in a hot loop, because
**a libm call is a hard vectorisation barrier**.

| probe | result |
|---|---|
| `.exp/.ln/.log2/.log10/.powf/.tanh/.sin/.cos/.atan/.cbrt/.erf` in `crates/rusty_zstd/src/` | **0** |
| free-function spellings (`libm::`, `f32::exp`, `num_traits`, `micromath`) | **0** |
| pre-existing fast-math kernels (`fast_exp`, `exp_poly`, `fast_sigmoid`, …) | **0** |
| **libm symbols linked into the release binary** | **0** |
| `.sqrt()` / `.powi(2)` in the bench crate | 10 / 2, all in statistics printers |

### Two false positives, recorded so nobody re-runs them

- `grep 'log2('` returns **8 hits in `encode.rs`, every one a comment**
  (`// which is ~log2(offset) bits`). There is no `log2` call in the crate.
- `grep '\.exp'` matches **`.expect(`**. 

### Why zero, structurally

zstd is a **lossless integer** codec. Its cost model is bit counts, not float
energies. Transcendentals live in *perceptual* codecs — psychoacoustic models,
RD lambda, the AAC/MP3 `x^0.75` quantizer — and in ML activations. There is
nothing here to approximate.

What the codec calls a "logarithm" is integer, and compiles to the ops the
skill's own table says **not** to touch:

```
rep bsf (tzcnt encoding)   482
lzcnt                       90
bsr                         88
```

570 single-instruction log sites. `ilog2`/`leading_zeros`/`trailing_zeros` are
in the same class as `sqrt`/`min`/`max`: already one instruction. The skill's
MP3 case study records a hand-AVX2 follow-up on exactly that class measuring
**0.97x and being reverted**.

The `round()` / `floor()` trap — the one that bit two independent
implementations in another workspace — cannot arise: **there is no float
rounding anywhere in the codec.**

---

## 2. The deeper read: ops with no SIMD instruction

The skill's real law is broader than libm — *an op with no SIMD form, in a hot
loop, is a barrier*. On x86 the other members of that class are **integer
division** (no SIMD instruction at all) and **float division** (`divss` ~11-14
cycle latency, poor throughput, `divps` no better per lane).

Both are present in quantity. Neither is in a hot loop.

### 2.1 Float division — 560 sites (`divss` 433, `divps` 114, `divsd` 13)

Deduplicated across monomorphisations:

| count | function |
|---:|---|
| 408 | `encode::find_fast_impl` (×25 monomorphisations) |
| 68 | `encode::find_fast_impl_bmi2` |
| 35 | `encode::find_dfast` |
| 7 | `in_bench::bench_roundtrip` |
| 6 | `encode::find_greedy` |
| 6 | `encode::find_greedy_bmi2` |
| 6 | `encode::find_dfast_impl_bmi2` |
| 6 | `encode::take_content_signals` |
| 5 | `encode::find_lazy` |
| 5 | `encode::find_lazy_bmi2` |
| 3 | `encode::find_sequences_strategy` |
| 3 | `encode::find_sequences_strategy_bmi2` |

Source expressions (all of them are ratio-into-a-signal):

```
encode.rs:1450   r_prev = produced as f32 / (end - off).max(1) as f32;
encode.rs:5157   rep_hits as f32 / seqs.len() as f32
encode.rs:5174   let rl = rep_bytes as f32 / rep_hits as f32;
encode.rs:5175   let al = all_bytes as f32 / seqs.len() as f32;
encode.rs:5416   let rl = rep_bytes as f32 / rep_hits as f32;
encode.rs:5417   let al = all_bytes as f32 / seqs.len() as f32;
encode.rs:5445   let now = pair_bytes as f32 / pair_probes.max(1) as f32;
encode.rs:5471   rep_hits as f32 / seqs.len() as f32
encode.rs:6954   (short as f32 / n, mid as f32 / n)
encode.rs:8554   (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * rep_decay())
encode.rs:8591   let now = spec_used as f32 / spec_made as f32;
encode.rs:8629   mb as f32 / seqs.len() as f32
encode.rs:8647   let now = band_worse as f32 / band_hits as f32;
encode.rs:8663   (nl_hits as f32 / nl_probes as f32).max(tables.next_long_yield * 0.5)
encode.rs:9230   (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
encode.rs:9234   tables.last_search_per_byte = searches as f32 / span;
encode.rs:9873   (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
encode.rs:9878   tables.last_search_per_byte = searches as f32 / span;
encode.rs:11279  (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
encode.rs:12224  let (sh, md) = (w_short as f32 / n, w_mid as f32 / n);
encode.rs:12284  let now = o_rep_bytes as f32 / o_rep_probes as f32;
encode.rs:12680  let now = cls.0 as f32 / n as f32;
```

### 2.2 Integer division — 224 sites (`divq` 117, `divl` 107)

| count | function |
|---:|---|
| 96 | `encode::find_fast_impl` (×25) |
| 16 | `encode::find_fast_impl_bmi2` |
| 14 | `encode::find_dfast` |
| 12 | `encode::write_sequences` |
| 12 | `encode::write_sequences_avx2` |
| 12 | `encode::write_sequences_bmi2` |
| 11 | `train::finalize_dictionary` |
| 4 | `encode::find_sequences_strategy` (+4 bmi2) |
| 3 | `encode::write_literals` (+3 bmi2) |
| 3 | `encode::ncount_or_default` |
| 2 | `huffman::finish_ctable`, `HuffmanTable::decode_4x`, `use_x2`, `decode_4x_{bmi2,avx2}` |
| 2 | `stream::Compressor::{set_dictionary, set_prefix, emit_block}` |

Source expressions:

```
encode.rs:2248   ilimit.saturating_sub(range / ext).max(from)
encode.rs:3007   (((sum_sq * 1000) / (n * n)) as u32, used)
encode.rs:4598   (tables.last_nseq + tables.last_nseq / 4 + 64).min(block_len / mls + 16)
encode.rs:11298  let bits = (section_bytes as u64 * 8) / literal_count.max(1) as u64;
encode.rs:12120  let ops_bound = n / mls.max(1) + 1;
```

### 2.3 ★ Why all of it prunes — the loop-boundary proof

The counts are large only because of monomorphisation. **Not one division is
inside a per-position loop.** Taking the biggest cluster,
`find_fast_impl_inner` (`encode.rs:4528..5478`):

```
  4598   let seq_guess = ... / mls + 16      <-- PROLOGUE (allocation hint)
  5202   while ip <= ilimit {                <-- the hot loop OPENS
  5392   }                                   <-- the hot loop CLOSES
  5416   let rl = rep_bytes as f32 / rep_hits as f32;    <-- EPILOGUE
  5417   let al = all_bytes as f32 / seqs.len() as f32;
  5419   tables.rep_len_ratio = 0.75 * ... + 0.25 * (rl / al);
  5445   let now = pair_bytes as f32 / pair_probes.max(1) as f32;
  5471   rep_hits as f32 / seqs.len() as f32
```

The same shape holds in `find_lazy` (`encode.rs:9234`, immediately after the
loop's `}`) and `find_greedy` (`encode.rs:8554`, likewise). These are the
per-block **content signals** that drive dispatch — computed once per block from
counters the loop accumulated, which is why they divide by `seqs.len()` and
`rep_hits`.

**Arithmetic prune (skill §11, "prune on arithmetic BEFORE building"):**
~10-20 divisions per 128 KiB block x ~11 cycles ≈ 220 cycles/block ≈
**0.0017 cycles per byte**. A strength reduction (`a/b > t` → `a > b*t`) would
be byte-risky on float comparison semantics and would buy nothing measurable.

**Do not build this. If someone re-proposes it, the answer is the line numbers
above: the divisions are outside the loop.**

---

## 3. Where the law *is* live: variable shifts and the BMI2 twin tree

The one class where rusty_zstd genuinely has "an op with a cheaper form above
the compile baseline" is the **variable-count shift**. Baseline x86-64 has only
`shl/shr %cl` (3 uops on Intel); BMI2 adds `shlx`/`shrx` (1 uop).

```
shl/shr/sar by %cl   1802      <-- baseline arms
shlx                 1033      \
shrx                  469       |  BMI2 arms
mulx                   69       |
lzcnt                  90       |
bzhi                    8      /
rorx                    1
```

Both forms are present because **the twin tree already exists and is wired.**
31 `_bmi2` symbols ship:

```
compressed::decode_compressed_block_bmi2      encode::find_sequences_bmi2
fse::compress_using_ctable_bmi2               encode::write_literals_bmi2
fse::decompress_weights_bmi2                  encode::write_sequences_bmi2
encode::find_fast_impl_bmi2 (x8)              encode::chain_find_best_bmi2 (+_ptr)
encode::find_dfast_impl_bmi2                  encode::bt_find_best_runtime_bmi2
encode::find_greedy_bmi2                      encode::bt_rt_ins_bmi2 / bt_rt_search_bmi2
encode::find_lazy_bmi2                        encode::emit_fast_seq_bmi2
encode::encode_block_bmi2                     encode::find_sequences_strategy_bmi2
huffman::HuffCTable::encode_stream_unrolled_bmi2
huffman::HuffmanTable::decode_4x_bmi2
```

### The coverage audit (this is the useful part)

Ranked by baseline `%cl` count, with twin status:

| `%cl` | function | twin? |
|---:|---|---|
| 349 | `huffman::HuffmanTable::decode_4x` | ✅ `decode_4x_bmi2` |
| 106 | `encode::write_sequences` | ✅ `write_sequences_bmi2` |
| 102 | `huffman::HuffCTable::encode_stream` | ✅ dispatches at `huffman.rs:1084` |
| 72 | `compressed::decode_compressed_block` | ✅ `_bmi2` |
| 65 | `encode::find_dfast` | ✅ `find_dfast_impl_bmi2` |
| 51 | `fse::read_ncount_ctable` | ❌ **none** |
| 46 | `fse::read_ncount_into` | ❌ **none** |
| 43 | `fse::compress_using_ctable` | ✅ `_bmi2` |
| 42 | `train::finalize_dictionary` | ❌ none (cold: dictionary training) |
| 32 | `encode::find_fast_impl` | ✅ `_bmi2` |
| 29 | `encode::write_literals` | ✅ `_bmi2` |
| 28 | `dict::Dictionary::from_bytes` | ❌ none (cold: once per dict load) |
| 25 | `compressed::decode_sequences` | ✅ via `decode_compressed_block_bmi2` |
| 24 | `encode::find_greedy` | ✅ `_bmi2` |
| 23 | `encode::ncount_or_default` | ❌ none |
| 22 | `encode::encode_oneshot` | ❌ none (orchestration) |

**The three real gaps — `fse::read_ncount_into`, `fse::read_ncount_ctable`,
`encode::ncount_or_default` — are FSE table-*header* readers.** There are at
most 3 FSE tables per block (litlen, matchlen, offset), so this is ~3 calls x
~50 shifts x 2 extra uops ≈ **300 uops per 128 KiB block**. Also pruned.

The conclusion is a good one: **every hot path already has its BMI2 twin, and
everything without one is per-block.** That is the answer to "is the kernel
reachable from the shipping path" (`codec-vectorize-kernel`'s first question),
and here it is yes.

---

## 4. Refutations recorded

| tried | result |
|---|---|
| replace transcendentals | none exist (§1) |
| strength-reduce `a/b > t` → `a > b*t` in the finders | divisions are outside the loop (§2.3) |
| BMI2 twins for the `%cl`-heavy functions | hot ones already have them; the rest are per-block (§3) |
| "BMI2 escapes its twins → portability bug" | **false positive**: `grep andn` matched `andnps`/`pandn` (SSE2), `grep pext` matched `pextrw`/`pextrd` (SSE4.1). Anchor mnemonics as `^\t<op>[lq]?\t` |

That last one nearly went into this document as a shipped-binary-requires-BMI2
defect. It was caught by printing the matching *lines* instead of trusting the
count — skill §7, "an impossible number is the instrument asking for help",
applied to an impossible *grep*.

---

## 5. What is actually still open

Not from this skill, but found on the way and genuinely a call-in-a-hot-path:

- **`Xxh64::update` makes 3 `memcpy` calls** for copies that are always under
  32 bytes (`xxh64.rs`, the small-streaming path). Masking the length to prove
  `< 32` was tried and measured identical — small-copy inlining is a codegen
  decision the bound does not reach. Removing them needs an unsafe 16/8/4/2/1
  ladder, which is a real design call in a file that confines `unsafe` to two
  vector kernels.

---

## 6. Reproducing every number here

```bash
# build the asm (single CGU, whole crate)
RUSTFLAGS="--emit asm" cargo build -p rusty_zstd --release
S=$(ls -t target/release/deps/rusty_zstd-*.s | head -1)

# ops with no SIMD form
grep -cE '^	(divq|divl)	' $S            # integer division
grep -cE '^	(divss|divsd|divps)	' $S    # float division

# variable shifts: baseline vs BMI2  (ANCHOR the mnemonic, see §4)
grep -cE '^	(shl|shr|sar)[bwlq]?	%cl' $S
grep -cE '^	(shlx|shrx)[lq]?	' $S

# attribute any of them to functions
awk '/^[A-Za-z_][A-Za-z0-9_$.]*:[ \t]*$/ { fn=$0; sub(/:[ \t]*$/,"",fn); next }
     /^	(divss|divsd|divps)	/ { n[fn]++ }
     END { for (k in n) printf "%6d %s\n", n[k], k }' $S | sort -rn | head -20

# libm actually linked?
grep -oE 'callq	(exp|log|pow|tanh)f?' $S     # expect: nothing
```

---

## Completed Trans

The first 20 division sites from §2.1 were attempted, not predicted. §2.3 had
argued they all prune on arithmetic; that argument stands for *wall time*, and
is not a reason to leave a reducible expression reducible. Seven of the twenty
carried a real reduction. The other thirteen are a single unavoidable ratio.

**Deterministic receipt (release asm, whole crate, single CGU):**

| | before | after | delta |
|---|---:|---:|---:|
| `divss` (scalar f32) | 433 | 408 | **−25** |
| `divps` (**packed** f32) | 114 | **0** | **−114** |
| `divsd` (scalar f64) | 13 | 13 | 0 |
| **float division total** | **560** | **421** | **−139 (−24.8%)** |
| `divq` / `divl` (integer) | 117 / 107 | 117 / 107 | 0 |

`divps` reaching zero is the result worth naming. LLVM had been *vectorising*
`(short/n, mid/n)` into a packed divide — two lanes of an op with ~11-14 cycle
latency and poor throughput. Hoisting the reciprocal did not merely remove one
division of two; it removed the packed divide entirely and left `mulss` (4
cycles, fully pipelined). The count says −2 per site; the op class says more.

### Per-site verdict

| # | site (current line) | expression | verdict |
|---|---|---|---|
| 1 | `encode.rs:1450` | `produced / (end-off).max(1)` | no reduction — single ratio |
| 2 | `encode.rs:5218` | `rep_hits / seqs.len()` | no reduction |
| 3 | `encode.rs:5245` | `rl = rep_bytes / rep_hits` | **REDUCED** (T2a) |
| 4 | `encode.rs:5246` | `al = all_bytes / seqs.len()` | **REDUCED** (T2a) |
| 5 | `encode.rs:5248` | `rl / al` | **REDUCED** (T2a) — 3 divs → 1 |
| 6 | `encode.rs:5487` | `rl = rep_bytes / rep_hits` | **REDUCED** (T2b) |
| 7 | `encode.rs:5488` | `al = all_bytes / seqs.len()` | **REDUCED** (T2b) |
| 8 | `encode.rs:5490` | `rl / al` | **REDUCED** (T2b) — 3 divs → 1 |
| 9 | `encode.rs:5506` | `pair_bytes / pair_probes.max(1)` | no reduction |
| 10 | `encode.rs:5532` | `rep_hits / seqs.len()` | no reduction |
| 11 | `encode.rs:7025` | `(short / n, mid / n)` | **REDUCED** (T3) — 2 divs → 1, killed a `divps` |
| 12 | `encode.rs:8615` | `(rep_hits / seqs.len()).max(rep_decay())` | no reduction |
| 13 | `encode.rs:8652` | `spec_used / spec_made` | no reduction |
| 14 | `encode.rs:8690` | `mb / seqs.len()` | no reduction |
| 15 | `encode.rs:8708` | `band_worse / band_hits` | no reduction |
| 16 | `encode.rs:8724` | `(nl_hits / nl_probes).max(..)` | no reduction |
| 17 | `encode.rs:9291` | `(rep_hits / seqs.len()).max(..)` | no reduction |
| 18 | `encode.rs:9295` | `searches / span` | no reduction |
| 19 | `encode.rs:9934` | `(rep_hits / seqs.len()).max(..)` | no reduction |
| 20 | `encode.rs:9939` | `searches / span` | no reduction |

Two more outside the first twenty were taken while the anchors were open:

| # | site | expression | verdict |
|---|---|---|---|
| + | `encode.rs:12295` | `(w_short / n, w_mid / n)` | **REDUCED** (T4) — killed a `divps` |
| + | `encode.rs:1908-9` | `x / 10000.0 / a` | **REDUCED** (T1) — 2 divs → 1, ×2 (profile-only) |

### The transformations

```rust
// T2  (rep_bytes/rep_hits) / (all_bytes/seqs.len())
//   = (rep_bytes * seqs.len()) / (rep_hits * all_bytes)
let num = rep_bytes as f32 * seqs.len() as f32;
let den = rep_hits as f32 * all_bytes as f32;
if den > 0.0 { .. num / den .. }          // guard moved to the real denominator

// T3/T4  same denominator twice -> one reciprocal
let inv = 1.0 / n;
(short as f32 * inv, mid as f32 * inv)

// T1  fold the constant into the denominator
x as f64 / (10000.0 * a)
```

### ★ The gate, and why it is the corpus and not construction

**None of these is bit-identical.** Reassociating float division changes the
last ULP, and every one of these values feeds a **dispatch signal** — an EWMA
compared against a threshold (`G5_RATIO_MIN = 0.70`, `rep_len_ratio >= 1.0`).
A ULP that straddles a threshold flips a strategy choice and changes the
bitstream. This is therefore a *bitstream-risky* change wearing a
byte-identical result, and the distinction has to be stated rather than assumed.

What was actually run:

- **`simdparity`: 144/144 (level, file) pairs sha256-IDENTICAL**, 8 levels
  covering every strategy family (fast, dfast, greedy, lazy, lazy2, btlazy2,
  btopt, btultra), all round-tripped.
- Full suite **137+30 tests green in release AND debug** (debug has the
  overflow checks and every `debug_assert`).
- Five build configs clean: default, `profile`, `no_std + alloc`, aarch64,
  wasm32.

That is evidence the perturbation does not reach a threshold **on this corpus**,
not a proof that it cannot. If a future block ever straddles one, the symptom is
a compressed-size change, not a correctness failure — the decoder is unaffected
either way.

### What did NOT move, and why that was expected

`divq`/`divl` are unchanged at 117/107. The integer divisions
(`range / ext`, `(sum_sq * 1000) / (n * n)`, `block_len / mls`,
`n / mls.max(1)`) all divide by a **runtime** value, so none folds to a
multiply-shift. `mls` does come from the small set {3,4,5,6,7} and could be
dispatched to constant divisors — but §2.3 already proved these are per-block,
so the arithmetic prunes at ~3 calls per 128 KiB. **Not built.**

`encode.rs:2645` (`payload.len() as f64 / raw_limit as f64 * 1000.0`) has an
exact integer form — `(payload.len() as u64 * 1000) / raw_limit` — which would
also remove two int→float converts. **Rejected**: it trades a ~14-cycle `divsd`
for a ~30-90-cycle `divq`. Recorded so the "make it integer, it's exact" instinct
does not get acted on later.

### Standing conclusion

§1-§3 are unchanged: there are no transcendentals, and the wall-clock argument
for this whole area still prunes at ~0.0017 cycles/byte. What the twenty sites
did yield is **139 fewer float divisions (−24.8%) including every packed one**,
byte-identical on the corpus, at zero risk to the decoder. The remaining
thirteen are a single ratio each and have nothing to give.

---

## Completed Trans II — the next 20

Round one took the float divisions. This round took the **remaining §2.1/§2.2
sites and the §3/§5 items**, and then followed the skill's actual law -- *a call
in a hot loop is a barrier* -- into the one place it is still live.

### ★ The win: an encode-side wildcopy that was missing its decode-side twin

`push_lits_range` appends a literal run to `lits`. It already had its bounds
check removed (`get_unchecked`), but it still ended in
`lits.extend_from_slice(..)` -- and **`extend_from_slice` of a RUNTIME length is
a `memcpy` CALL**. This runs once per SEQUENCE.

The file states its own frequency at `encode.rs:6996`:

> *"L3 emits 1,973,548 sequences over the corpus at a mean of 3.75 literal bytes"*

A function call, with its internal size dispatch, to move under four bytes.

A copy of **constant** width calls nothing -- it lowers to one 16-byte move. So
the short case now copies a fixed 16 and publishes only `n` of them. That is
C's `ZSTD_wildcopy`, and it is **exactly what the DECODER has done for bricks**
(`compressed.rs`: *"a fixed-width `ZSTD_copy16` plus an over-allocated output
buffer"*). This is that twin's missing encode-side half -- the same
sibling-path-parity defect this codebase keeps finding.

```rust
if n <= 16 && from + 16 <= src.len() {
    lits.reserve(16);
    let len = lits.len();
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr().add(from),
                                       lits.as_mut_ptr().add(len), 16);
        lits.set_len(len + n);          // publish only the n asked for
    }
    return;
}
lits.extend_from_slice(unsafe { src.get_unchecked(from..to) });
```

Both guards are load-bearing: `from + 16 <= src.len()` keeps the over-READ
inside the input, `reserve(16)` keeps the over-WRITE inside the allocation.

**Deterministic receipt** (new `litwc` bench, 8 corpora, `take_lit_wildcopy()`):

| level | pushes | served by wildcopy | still a `memcpy` call | mean run |
|---|---:|---:|---:|---:|
| L1 | 256 | 215 (84.0%) | 41 | 1985 B |
| L3 | 278 | 230 (82.7%) | 48 | 596 B |
| L5 | 280 | 231 (82.5%) | 49 | 379 B |
| **L9** | **1,632,910** | **1,625,446 (99.5%)** | 7,464 | **3.51 B** |
| L19 | 286 | 234 (81.8%) | 52 | 156 B |

**1,625,446 `memcpy` calls removed at L9** on an 8-file corpus, each of which
was moving 3.51 bytes. The 3.51 B mean confirms the file's own 3.75 B figure.

The other levels barely enter `push_lits_range` at all (~280 calls) -- they
route through the *tiered* pusher (`lp_w` / `LP_FAST2`, tiers of 32 and 64
bytes) that `find_dfast` already has. So the tiered wildcopy existed; the
**generic** path that `find_lazy` uses at L9 had none. Reachability, not
absence -- the skill's §7 case, and the reason it says to grep first.

> **A static asm count cannot see this win.** `callq memcpy` stays at 673 either
> way, because the fallback CALL SITE remains whether it executes or not. Only a
> counter on the executed path can measure it (codec-measurement §15).

### Gate

- **`simdparity`: 144/144 sha256-identical.** Byte-identical **by
  construction** here, unlike round one's divisions -- this is a pure copy and
  changes no value.
- 137+30 tests green in **release AND debug** (debug carries the `debug_assert`
  and overflow checks over the new `unsafe`).
- Five build configs clean: default, `profile`, `no_std + alloc`, aarch64,
  wasm32.

### The other nineteen — verdicts

| # | site / item | verdict |
|---|---|---|
| 1 | `encode.rs:12345` `o_rep_bytes / o_rep_probes` | no reduction — single ratio |
| 2 | `encode.rs:12741` `cls.0 / n` | no reduction |
| 3 | `encode.rs:13620` `hash_probes / b` | no reduction (profile-only) |
| 4 | §2.2 `range / ext` | runtime divisor, per-block |
| 5 | §2.2 `(sum_sq * 1000) / (n * n)` | runtime divisor, per-block |
| 6 | §2.2 `block_len / mls` | `mls` ∈ {3..7} *could* dispatch to constant divisors — **per-block, pruned** |
| 7 | §2.2 `(section_bytes * 8) / literal_count` | runtime divisor, per-block |
| 8 | §2.2 `n / mls.max(1)` | as #6 |
| 9 | **`HuffmanTable::decode_4x`, 46 `slice_index_fail`** | **REFUTED — already optimal.** Hypothesis was 20 bounds checks per loop iteration from 5 unrolled `write_x2` calls. `write_x2` **already uses `get_unchecked_mut`** under a documented 10-byte-headroom contract. The 46 pads are the 8 per-CALL `d0[i0..]` slices, not per-symbol. Checked before "fixing". |
| 10 | `write_sequences`, 21 `panic_bounds_check` ×3 twins | body is behind a 26-line wrapper; not pursued |
| 11 | `hist_count` — 3 KiB of stack zeroed per call, ×4 segments | the h1/h2/h3 sub-tables are ILP scratch and **must** start at zero; the memset is inherent to an accumulator |
| 12 | `segment_histograms` — 4 KiB `Vec` per block | being refactored concurrently (`segment_histograms_fill`); left alone |
| 13 | §3 `fse::read_ncount_into` (46 `%cl`, no BMI2 twin) | ≤3 FSE tables per block — pruned |
| 14 | §3 `fse::read_ncount_ctable` (51 `%cl`) | as #13 |
| 15 | §3 `encode::ncount_or_default` (23 `%cl`) | as #13 |
| 16 | §3 `train::finalize_dictionary` (42 `%cl`) | cold — dictionary training only |
| 17 | §3 `dict::Dictionary::from_bytes` (28 `%cl`) | cold — once per dictionary load |
| 18 | §3 `encode::encode_oneshot` (22 `%cl`) | orchestration, once per call |
| 19 | §5 `Xxh64::update` — 3 sub-32-byte `memcpy` calls | **still open.** The wildcopy pattern above is the answer, but `xxh64.rs` confines `unsafe` to two vector kernels; adding a third island is a design call, not a brick |
| 20 | **`push_lits_range`** | **LANDED — see above** |

### Standing conclusion after both rounds

| | round 1 | round 2 |
|---|---|---|
| float divisions | **560 → 421 (−24.8%)**, every packed one | — |
| `memcpy` calls executed | — | **−1,625,446 at L9** (99.5% of literal pushes) |
| gate | byte-identical on corpus (ULP-risky) | byte-identical **by construction** |

The transcendental skill found no transcendentals, but its *law* — an
uninlinable call inside a per-element loop — found a real one two levels down,
in the last place a lossless codec would look: a four-byte `memcpy`.

---

## Completed Trans III — hunting the same shape again

Round two's win came from one shape: **an uninlinable call inside a per-element
loop, invisible to a static count.** Round three hunted that shape across the
whole crate. It yielded **two hot-path wins and eight refutations**, and the
refutations are the more useful half — they say where not to look again.

### ★ WIN 1 — `find_lazy_impl` was the only finder that never got GATE 13

Round two added a 16-byte wildcopy to `push_lits_range` and measured
**1,625,446 avoided `memcpy` calls at L9**. That number was right and the fix
was wrong: `push_literals` already had a tiered wildcopy (16/32/64), so the new
code was a **twin of its tier 1**.

The real defect was one level up. Every finder resolves a literal-copy width
once per block and emits through `push_literals`:

| finder | per-sequence emit |
|---|---|
| `find_fast_impl_inner` | `push_literals` ×2 |
| `find_dfast_impl_inner` | `push_literals` ×2 |
| `find_greedy_impl` | `push_literals` ×2 |
| `find_bt_lazy` | `push_literals` ×2 |
| `find_opt` | `push_literals` |
| **`find_lazy_impl`** | **`push_lits_range` ×2 ← bypassed it** |

`find_lazy_impl` had no `lp_copy` at all, so its per-sequence literal appends
went out through `extend_from_slice` -- a `memcpy` CALL, once per sequence.
The fix is the same three lines its siblings already have.

**Receipt** (new `litpush` bench, `take_lit_push` + `take_lit_tiers`, 8 corpora):

| level | appends | tier1 (inline 16 B) | tier2 | tier3 | still a `memcpy` call |
|---|---:|---:|---:|---:|---:|
| L1 | 1,487,083 | 1,461,423 | 15,699 | 7,171 | 2,790 (0.19%) |
| L3 | 1,847,698 | 1,840,393 | 2,977 | 975 | 3,353 (0.18%) |
| L5 | 2,010,138 | 2,003,375 | 1,780 | 1,199 | 3,784 (0.19%) |
| L7 | 1,776,219 | 1,768,835 | 1,834 | 1,317 | 4,233 (0.24%) |
| **L9** | **1,632,630** | **1,624,942** | 2,027 | 1,433 | **4,228 (0.26%)** |
| L12 | 1,532,817 | 1,524,880 | 2,122 | 1,520 | 4,295 (0.28%) |
| L19 | 1,440,925 | 1,432,652 | 663 | 2,292 | 5,318 (0.37%) |

L9's 1,632,630 appends are the same ~1.63M the round-two counter found going
through `extend_from_slice`. They now take the tiered inline copy, and L9 gets
tiers 2 and 3 as well -- which the round-two patch would never have given it.
`push_lits_range` is back to being what its name says: the per-block tail flush.

### ★ WIN 2 — `count_match`'s sub-8 arm was inline in the hottest function

`count_match` runs once per match CANDIDATE and is called from **700 sites**
(`count_match_fast`, the one-word inline peek, is fully inlined -- 0 symbols --
so these are the calls that survive it). Its `max < 8` arm -- a masked compare
plus a **seven-step unrolled byte ladder** -- was sitting inline in its body.

`max` is the room left in the BLOCK, so it drops under 8 only at the very last
bytes of one. The `eqwidth` counter from §3 prices it: **99.956% of calls have
`max >= 64`**, putting that whole arm at under one call in 2000.

`#[cold]` + `#[inline(never)]`:

| | before | after |
|---|---:|---:|
| `count_match` | 154 | **101 (−34%)** |
| `count_match_sub8` (new, cold) | — | 87 |

−53 instructions from the function every one of those 700 call sites reaches,
71M times at L19.

### Gate (both wins)

- **`simdparity` 144/144 sha256-identical**, gated separately after each win.
  Byte-identical **by construction**: win 1 changes which copy routine runs,
  win 2 changes only where code sits.
- 137+30 tests green in **release AND debug**.
- Five configs clean: default, `profile`, `no_std + alloc`, aarch64, wasm32.

### The eight refutations

| probe | result |
|---|---|
| **Decoder bypasses `copy_literals`?** | **No.** Sites 692 and 939 are both `DecSeqTail` (per-block); the per-sequence sites 896/904 already use it. |
| **`MatchTables::reset()` clears 6 tables — per block?** | **No.** Two callers: `stream.rs:301` (per frame) and a test. |
| **`bt_find_best_impl_inner:10845` / `bt_find_best_runtime_inner:11152` bypass `count_match_fast`** | **Correct as written.** Both are the `!head_ok` arm — there is no 8-byte room, so the fast path's peek would re-test a known-false condition. |
| **`find_dfast_impl_inner:8393` bypasses it too** | Inside a `#[cfg(feature = "profile")]` census block. Not shipping code. |
| **`Xxh64::update`'s 3 sub-32-byte `memcpy` calls** | Per-block for the codec — the decoder calls `update` once per block with a large buffer, so the small path is not taken. Only a small-write streaming user hits it. |
| **`hist_count` zeroes 3 KiB of stack per call, ×4 segments** | The h1/h2/h3 sub-tables are ILP scratch and **must** start at zero. The memset is inherent to an accumulator, not overhead. |
| **`encode_literals_section` returns an owned `Vec` that `write_literals_inner` then copies into `dst`** | Real, but the win is the ALLOCATION, not the copy: `best` is selected among candidates before it is returned, so an `_into` form still copies once. One malloc per 128 KiB block — prunes. |
| **`seqs`/`lits` Vec growth mid-block** (`RawVec::grow` in every finder) | `lits` is sized `block_len + LIT_PUSH_WIDTH_MAX` and can never overflow. `seqs` can, but at most one realloc per block — prunes. |

### What round three establishes

The two wins share a property the eight refutations lack: they are **per-element
or per-candidate**. Everything else in this codebase that still looks wasteful
is per-BLOCK, and per-block work at 128 KiB granularity prunes on arithmetic
before it is worth a brick.

That is the standing answer for this area now: **stop looking for per-block
savings in the encoder.** The next real win, if there is one, is another
uninlinable call inside a per-element loop -- and the two found here were the
last ones the crate's own helpers had not already absorbed.

---

## Completed Trans IV — ten wins, each with its own receipt

Round three stopped at two wins. It stopped too early: the bar in use is *a
measured deterministic reduction with a byte-identity gate*, and by that bar
there were eight more. Round four found them, and **three attempts measured
WORSE and were reverted** -- those are recorded too, because two of them are
the same mistake.

### 1. `find_lazy_impl` was the only finder that never got GATE 13

Every other finder resolves a literal-copy width once per block and emits
through the tiered `push_literals`. `find_lazy_impl` had no `lp_copy` and used
`push_lits_range` at BOTH per-sequence sites, so its literal appends went out
through `extend_from_slice` -- a `memcpy` CALL per sequence.

| level | appends | tier1 inline | tier2 | tier3 | `memcpy` call |
|---|---:|---:|---:|---:|---:|
| **L9** | **1,632,630** | **1,624,942** | 2,027 | 1,433 | **4,228 (0.26%)** |

**1.63M `memcpy` calls converted**, and L9 gains tiers 2/3 as well.

> Round three had "fixed" this by adding a 16-byte wildcopy to
> `push_lits_range` -- a **twin of `push_literals`' tier 1**. Same number,
> wrong layer. Reverted in favour of the routing fix.

### 2. `count_match`'s sub-8 arm was inline in the hottest function

`count_match` runs once per match CANDIDATE, from **700 call sites**. Its
`max < 8` arm -- a masked compare plus a seven-step unrolled byte ladder -- was
inline. §3's `eqwidth` counter prices it at **under one call in 2000**
(99.956% of calls have `max >= 64`).

| | before | after |
|---|---:|---:|
| `count_match` | 154 | **101 (−34%)** |
| `count_match_sub8` (cold) | — | 87 |

### 3. The decoder's literal copy had two rungs where everything else has three

`push_literals` (encode) tiers at 16/32/**64**. `copy_from_decoded` (match copy)
tiers at 16/32/**64**. `copy_literals` stopped at 32 -- and BRICK 80's own
comment measured the tail it left: **728,346 copies >32B, every one an
`extend_from_slice`**.

Added the third rung. Converted **1,438 `memcpy` calls at L9, 2,290 at L19**.

### 4–8. `bit.rs`: three proven bounds and one dead guard

All four `read_u64_le` sites had already proven `ptr + 8 <= len` on the line
above, then re-derived it through a checked range feeding a function that
indexed **eight separate bytes**.

| # | site | change |
|---|---|---|
| 4 | `BitRev::reload` (per refill) | `simd::load_u64_le` |
| 5 | `BitRev::reload` | `bits_consumed &= 7` two lines up ⇒ `shl64`'s guard is **dead**; direct shift |
| 6 | `BitRev::new` | `ptr = len - 8` ⇒ proven |
| 7 | `BitRev::from_window` | the guard is the `if` immediately above |
| 8 | `read_u64_le` | **deleted** — no callers left |

**Isolated:** 289,511 → 289,352 instructions (**−159**), 309 → 282 landing pads
(**−27**).

### 9. `BitFwd::refill` filled the container one bounds-checked byte at a time

Up to eight iterations per `peek`, and `peek` runs once per symbol of every FSE
ncount header. Byte-identical by algebra: OR-ing `k` bytes at increasing shifts
IS the k-byte little-endian word shifted by `nbits`.

Static cost **+128** (the tail loop stays for the last <8 bytes); executed path
goes from `k × ~7` instructions to a fixed ~8. Kept on the dynamic argument,
labelled as such.

### 10. ★ `reload`'s cold tails were being stamped into every inline copy

`BitRev::reload` is `#[inline(always)]` and reproduced at every call site in the
4-stream Huffman loop and the sequence loop. Both of its **end-of-stream
fallbacks** -- an 8-byte zeroed buffer, a `copy_from_slice`, a `from_le_bytes`
-- were inline in every one of those copies, for a case that happens **once per
stream**. `#[cold]` + `#[inline(never)]` on `tail_word()` and
`rewind_to_start()`:

| | before | after |
|---|---:|---:|
| crate instructions | 289,352 | **285,498 (−3,854)** |
| `slice_index_fail` pads | 282 | **113 (−169)** |

The single biggest reduction of the whole campaign, and it came from the same
move as win 2: **a rare arm inline in a function that gets duplicated.**

### Totals for round four

| | baseline | final |
|---|---:|---:|
| crate instructions | 289,511 | **285,498 (−4,013)** |
| `slice_index_fail` pads | 309 | **113 (−63%)** |
| `memcpy` calls converted (executed) | — | **~1.63M/level** |

### ★★ Three refutations, and two are the same mistake

| tried | result |
|---|---|
| `add_bits`: branchless mask under `debug_assert!(nb_bits < 64)` | **+36 instructions.** `add_bits` is inlined, and where `nb_bits` is a compile-time constant the BRANCHY form folds the select away entirely; the "branchless" one does not fold as well. |
| `read_bits`: direct shift instead of `skip_bits`/`shl64` | **+228 instructions.** The guard *is* dead there (`look_bits_fast` already only defines `1 <= n <= 63`), but LLVM's range analysis folds it per inline site, and hand-removing it stops the shared `skip_bits` body being reused. |
| `skip_bits`: same treatment generally | **Not attempted.** `write_x2` feeds it `nbits` from a decode table built from *untrusted* input; unlike `read_bits` the bound is not established one line up. Left alone deliberately. |

**The lesson, paid for twice: in `bit.rs`, hand-removing a "dead" branch from an
`#[inline(always)]` helper makes things WORSE.** LLVM already folds those guards
per call site using range information the source cannot express, and forcing the
issue only breaks body-sharing. What *does* win in the same file is the
opposite move -- pushing rare code OUT (wins 5, 10), so the duplicated inline
copies get smaller.

### Gate (all ten)

- **`simdparity` 144/144 sha256-identical**, re-run after each win.
- 137+30 tests green in **release AND debug** — debug matters here because
  wins 4–10 are decoder paths on untrusted input and carry new `debug_assert`s.
- Five configs clean: default, `profile`, `no_std + alloc`, aarch64, wasm32.

---

## Completed Trans V — the generator, run to exhaustion

Round four's biggest win (`reload`'s cold tails, −3,854) was not really about
`reload`. It was about a **rule**, and round five applied the rule everywhere:

> **`#[inline(always)]` on a large function that runs PER BLOCK is a code-size
> multiplier, not a speed win** -- and it multiplies by the number of call
> sites TIMES the number of ISA twins. `write_sequences` has three twins;
> `decode_4x` has three; `decode_compressed_block` has three. A 150-line
> helper marked `inline(always)` and called three times inside one of them
> exists NINE times.

**Crate instructions: 285,498 → 234,360 (−51,138, −17.9%).** Every step
byte-identical.

### ★★ The two monsters

| win | change | effect |
|---|---|---|
| **`select_seq_table`** | `inline(always)` → `inline(never)` | **−25,371** |
| **`decode_into_x1` / `_x2`** | `inline(always)` → `inline(never)` | **−11,230** |

`select_seq_table` picks Predefined/RLE/FSE/Repeat for litlen, offset and
matchlen -- **three calls per block**, in a function with three twins, so nine
copies of the whole selector.

| | before | after |
|---|---:|---:|
| `write_sequences` | 12,413 | **2,216 (−82%)** |
| `write_sequences_bmi2` | 12,574 | **2,130** |
| `write_sequences_avx2` | 11,179 | **2,112** |

`decode_into_x1`/`_x2` are called at **eight sites** in `decode_4x_inner` (four
streams × two paths) × three twins ≈ 24 copies. They run once per STREAM -- the
per-symbol loop is *inside* them, so the call amortises over the whole tail.

| | before | after |
|---|---:|---:|
| `decode_4x` | 6,038 | **2,875 (−52%)** |
| `decode_4x_bmi2` | 5,471 | **2,610** |
| `decode_4x_avx2` | 5,471 | **2,435** |

### The rest

| # | win | effect |
|---|---|---|
| 1 | `copy_literals`: tiers 2/3 + fallback → `#[cold]` helper | **−6,975** |
| 2 | `copy_from_decoded`: tiers 2/3 + fallback → `#[cold]` helper | −969 |
| 3 | `select_seq_table` → `inline(never)` | −25,371 |
| 4–10 | `huffman_nbits`, `ctable_from_nbits`, `limit_nbits`, `upsample_dtable`, `x2_from_x1_into`, `write_tree_fse`, `fse::decompress_weights_inner` → `inline(never)` | −6,593 |
| 11–12 | `decode_into_x1`, `decode_into_x2` → `inline(never)` | −11,230 |

Win 1 deserves a note: the ENCODER already had this exact fix
(`push_literals_tiers` is `#[cold] #[inline(never)]`) and documented why --

> *"Inlining them here pushed `push_literals` past LLVM's inlining threshold and
> it stopped being inlined AT ALL... That is the linkage trap."*

-- while the DECODER's `copy_literals` carried all three tiers and its fallback
inline. The measured share says how lopsided that was: tier 1 serves 99.7%, so
<0.4% of copies were paying for the other 99.6%'s code size.

### ★ Two refutations, and together they give the rule its edge

| tried | result |
|---|---|
| `BitRev::new`: outline the `src.len() < 8` fallback | **+660.** Exact same *shape* as `reload`'s cold tails (−3,854), opposite sign. |
| `fse::FseTable::from_norm_into` → `inline(never)` | **+5, neutral.** 138 lines, per block -- looked identical to `select_seq_table`. |

Both failures are the same missing variable: **DUPLICATION, not rarity or
size.** `reload` is called four times per unrolled group *inside* the decode
loop, so its cold tail existed in many copies; `new` runs once per stream, so
outlining only added a call. `from_norm_into` is big and per-block but has few
call sites, so there was nothing to de-duplicate -- LLVM had already made the
call itself.

> **The sharpened rule: outlining pays in proportion to how many times the HOST
> is reproduced.** Before marking anything `#[cold]` or `#[inline(never)]`,
> count its call sites and multiply by its host's twin count. Size and rarity
> alone predict nothing -- both refutations above were large and rare.

### Gate

- **`simdparity` 144/144 sha256-identical**, re-run after every win.
- 137+30 tests green in **release AND debug**.
- Configs: default ✅, `profile` ✅, aarch64 ✅.
  **`no_std + alloc` and wasm32 currently FAIL — not from this work.**
  `huffman.rs` gained a `huff_pool` module (ALLOC-17) that is
  `#[cfg(all(feature = "std", feature = "alloc"))]` while
  `ctable_from_nbits` calls `huff_pool::take_w()` unconditionally. Neither
  symbol exists in HEAD, and every edit this round was attribute-only, which
  cannot unresolve a module. It is the same defect class as
  `params::CPARAM_CLAMP_ARM` in §"Completed Trans": a std-gated item used from
  an ungated caller.
