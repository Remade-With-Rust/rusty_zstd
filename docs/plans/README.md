# docs/plans

The campaign record. Every public number in this repository is traceable to a
board on one of these pages, and every reverted idea keeps its measurement here
so it is not re-litigated.

| Page | What it holds |
|---|---|
| [`rusty-zstd-mission.md`](rusty-zstd-mission.md) | The deployment plan: the format contract, the product surfaces, the milestones (M1-M7) and section 7's numeric exit bars |
| [`m7-anatomy.md`](m7-anatomy.md) | The standing speed/ratio boards vs facebook/zstd v1.5.7, all 18 corpora at L1 and L3, plus the per-stage share tables. **This is the page the README quotes** |
| [`m7-benchmark-repair.md`](m7-benchmark-repair.md) | How the speed instrument was found to be unsound, and what it took to repair it. Read before trusting any number here |
| [`m7-optimize-anatomy.md`](m7-optimize-anatomy.md) | The optimization campaign brick by brick, keeps and reverts alike |
| [`m7-encoder-whys.md`](m7-encoder-whys.md) | The six-whys descents on encoder unknowns |
| [`engine-matchfind.md`](engine-matchfind.md) | What the match-find campaign did to each of the nine finders |
| [`allocation-census.md`](allocation-census.md) | Where the allocations were, and which ones are gone |
| [`inline-execution.md`](inline-execution.md) | Inlining and outlining decisions, with the instruction counts behind them |
| [`fast-trans.md`](fast-trans.md) | The fast-path transform work |
| [`supercharge-engine.md`](supercharge-engine.md) | The engine-level plan the above descends from |

## A note on `gg-*` references

Source comments and some pages refer to `gg-matchfind.md`, `gg-Addendum.md` and
gate numbers from them ("Gate 9", "P1/gg-matchfind candidate signal"). Those are
the **Great Gate** content-adaptive-dispatch campaign documents, which are kept
internal and are not published here — the same policy `_greatgate/` has, and the
same one `remade_ffmpeg_rs` follows.

Nothing is hidden by that: the gates themselves are in the source, each with its
own comment explaining what it dispatches on and why, and their arms are
reachable through the benchmark harness. The `gg-*` names are provenance markers
for an internal planning document, not a dependency.
