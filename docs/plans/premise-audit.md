# The expired-premise audit

**Status:** instrument built (`tools/premise_audit.py`), first pass run, first
finding landed. This page is the standing procedure.

## The failure mode

An `#[inline(always)]`, a `#[target_feature]` twin, a `#[cold]` — each is a
DECISION, and the comment beside it is a CLAIM about the body it applies to.

**Bodies move.** Every refactor that outlines shared work, collapses a
monomorphisation tree, or extracts a cold arm changes what is inside some other
function — and the decision attached to that function keeps its comment, its
attribute, and its authority, while its reason quietly leaves.

Eight of these have now been found. Every one was found **by accident**:

| decision | its stated premise | what had happened |
|---|---|---|
| `decode_literals` | "outlined, it ran baseline (transitive trap trace)" | kernels grew their own CPUID dispatch |
| `parse_ncount_into` | "so BMI2 callers get this loop in their ISA context" | its only caller became ISA-neutral |
| `encode_block_bmi2` | "the section packing carried 34 variable shifts" | packing moved to `write_sequences`/`write_literals` |
| `write_literals_bmi2` | "the table builders all carry variable shifts" | `from_norm` was outlined |
| `write_sequences_avx2` | "the per-SEQUENCE transcode lives in here" | it moved to `build_coded_pass` |
| `decode_4x_avx2` | a 17-twin sweep: 6,436 → 6,311 | its VEX work moved to two outlined arms |
| `decode_compressed_block_avx2` | "converts all 57 SSE to VEX, emits 71 ymm" | `decode_literals` was outlined |
| `decode_compressed_block_bmi2` | "the driver carried 100 variable shifts" | same |

The pattern is mechanical, so the audit should be too.

## The instruments, ranked by what has actually worked

`python tools/premise_audit.py [path/to/rusty_zstd-<hash>.s]`

### 1. Twin density — **the strongest**

For every `_bmi2`/`_avx2` symbol, count what its ISA actually converts against
what the duplicate costs: BMI2 ops, `%ymm`, vector *math* (not moves), VEX.

Verdicts it prints:
- **converts NOTHING** — retire. Six of the eight above were caught here.
- **ymm but no vector math** — the ymm are `vmovups` spill widening, not work.
  This is what `write_sequences_avx2` looked like: 98 ymm, 0 arithmetic.
- **thin** (>100 instrs per ISA op) — judgement, not a verdict. See below.

Two rules when reading it:
- Compare an avx2 twin against its **bmi2 sibling**, never the baseline. The
  file's own rule is *"enable avx2 where the instruction count DROPS; revert
  where it GROWS."*
- A twin's ISA density is only meaningful **after** you know its callees'
  dispatch. If every callee runs its own `has_bmi2()`, an outer twin is
  buying only what is left in its own body.

### 2. Inline census — body lines × call sites

Found `FseCTable::from_norm` (140 × 8), `read_table` (42 × 11),
`prime_tables` (223 × 5), `parse_ncount_into`. Caveats:
- The site count includes **tests**; check before acting.
- A big body at **setup frequency** (per dictionary, per stream, per block) is
  the win. A small body at per-position frequency is not — leave it inlined.
- A high site count is not a cost when the callee has no body: outlining
  `di_x1`/`di_x2` (36 apparent expansions) measured **+6**, because they are
  three-instruction dispatchers.
- **The census counts SOURCE lines, and source lines can compile to nothing.**
  `tap_block` reads as 31 lines at 6 sites; its body is `note_block_tap` +
  `encode_counts`, both no-ops outside `--features profile`, so it emits zero
  instructions in release and outlining it measured **0**. Before acting on a
  census row, confirm the function HAS a body in the shipping build --
  `grep -c '<name>' the .s` is enough.

### 3. Locative claims — **noisy, a reading list only**

Scans for comments asserting code *lives in* a body that no longer contains it.
It reports variables and cross-references as readily as premises. Kept because
it costs nothing to run, but nothing here is a verdict.

## Procedure

1. Build with `RUSTFLAGS="--emit asm"`, run the audit.
2. For each candidate, **read the comment** — it names its own premise.
3. Test that premise against the current asm, not against the comment.
4. If expired: retire, and **replace the comment with the measurement that
   retired it**, so the next audit starts from fact rather than folklore.
5. Gate: `simdparity` 144/144 byte-identical, release + debug suites, four configs.

## Open, with numbers — not taken

Five twins convert real but thin ISA work. Cost per ISA op converted:

| twin | instrs | ops | instrs/op |
|---|---:|---:|---:|
| `chain_find_best_bmi2` | 460 | 3 | 153 |
| `find_greedy_bmi2` | 1,234 | 10 | 123 |
| `find_lazy_bmi2` | 1,002 | 9 | 111 |
| `bt_find_best_runtime_bmi2` | 291 | 3 | 97 |
| `find_dfast_impl_bmi2` | 1,287 | 18 | 72 |

Retiring the worst three returns ~2,700 instructions for 22 shift encodings.
**Not taken:** those shifts sit in per-position loops at L5–L12, where the board
levers are, and `shr %cl` versus `shrx` is a decision the clock should make.
The remaining twins sit at 20–56 instrs/op and are defensible as they stand.

**This table expires too.** Re-run the audit after any refactor that outlines
shared work — that is precisely the event that empties a twin.
