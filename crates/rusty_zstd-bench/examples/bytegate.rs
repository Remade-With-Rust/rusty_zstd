//! DETERMINISTIC BYTE-IDENTITY GATE for the inline-execution bricks.
//!
//! Every speed brick in `inline-execution.md` except E1 must be byte-identical.
//! This compresses every corpus at every level, prints the size table, and folds
//! ALL compressed bytes into one 64-bit number. Run before and after a brick:
//! if GOLD moves, the brick changed the bitstream and is not byte-identical.
//! It also round-trips each frame, so it is a correctness gate as well.
//!
//! A count, not a clock -- same number on any machine at any load.
//!
//! GOLD HISTORY. This number is the campaign's identity anchor and only moves
//! when a bitstream-changing arm is deliberately flipped:
//!
//! ```text
//!   BE0071FB0CB0CED9   59,760,356 bytes   until 2026-08-27
//!   CAE84167220B70DA   59,841,188 bytes   DFAST_FILL_N_ARM -> start-only
//!   EA4E12B951B48F4A   59,852,335 bytes   dfast_bext ON + walk_first_max -0.15
//!   F72C7074A2240AF7   59,704,523 bytes   ROW finder AUTO on the lazy ladder
//!   269F0EC2BA6B8550   59,686,173 bytes   nl_dispatch ON + raised cut 24->48
//!   D8F9B47AD5DDD2AB   59,685,682 bytes   lazy/greedy/bt incompressible accel
//!   7FB4E822473412A3   59,685,682 bytes   source-sized hash on the Bt ladder
//!   2F6594F7EEDBD12B   59,680,638 bytes   source-sized hash on EVERY strategy
//! ```
//!
//! The 2026-08-27 move is +0.135% of total bytes and buys HALF the per-match
//! table fills at L1 and L3/L4 (`fillcut.rs`, and section 9 of
//! docs/plans/m7-anatomy.md). L5 and above are unaffected -- they do not fill
//! through this path.
//!
//! The 2026-08-28 move is +0.019% NET, and that small number is two much larger
//! ones cancelling -- see `shipboth.rs`, which boards them together:
//!
//!   * `dfast_bext` ON: **-1.276% at L3, -1.146% at L4**, zero corpora
//!     regressing, probe count flat. A pure win on the DEFAULT level's ladder.
//!   * `walk_first_max` -0.15: **+0.35% to +1.39% size for +5.2% to +38.1%
//!     encode throughput** at L5/L7/L9/L12. The one deliberate size-for-speed
//!     trade in this encoder. `RZSTD_WALK_FIRST_MAX=0.70` restores the old
//!     bitstream exactly at L7/L9.
//!
//! The 2026-09-08 move is **-147,812 bytes, -0.247%**, and is a pure ratio
//! gain -- no size was traded for anything. The row match finder existed,
//! round-tripped and defaulted OFF because THIS BOARD'S SIBLING measured it at
//! one input size. `rowboard.rs` caps every corpus at 8 MiB and reported L9
//! aggregate 1.0005x, a wash. Swept across caps the verdict is monotone in
//! source length and 8 MiB sits just past the crossover:
//!
//! ```text
//!   L9  256K 1.0059 | 512K 0.9882 | 1M 0.9882 | 2M 0.9894 | 4M 0.9944 | 6M 1.0018
//! ```
//!
//! A row holds the last 16 positions for its bucket where the chain held all
//! of them: DEPTH traded for RECENCY, and recent means SMALL OFFSETS, which
//! cost fewer bits. While the window is not full the row gives up almost no
//! depth and banks the offset saving. So the arm now defaults to AUTO and
//! fires only for Lazy/Lazy2 with a KNOWN source length in 512 KiB..2 MiB --
//! the band that wins at L7, L9 and L12 alike. Outside it, output is
//! byte-identical to the previous default, which is why only the in-band
//! cells of this table moved.
//!
//! In band it is a DOUBLE win: 1.1-1.3% smaller AND 2.5-8.5x fewer dependent
//! loads. Streaming keeps the chain -- the band is a source-length band and
//! streaming does not know the length.
//!//! The second 2026-09-08 move is **-18,350 bytes** and touches L3 only -- the
//! next-long probe COMMITS at `ip + 1`, so it can take a longer match at a
//! worse OFFSET and lose more in the offset code than it gains in the length
//! code. On `sao` it wins 468,072 match bytes and still costs 15,001
//! compressed bytes. `next_long_yield` cannot see that (x-ray's yield is 100x
//! lower and the probe HELPS it), but `band_worse / band_hits` measures the
//! offset trade directly and `nl_cut_for` already dispatched on it -- the
//! dispatch had simply never been switched on, so the sweep that tuned its
//! 0.60 bar was tuning a threshold nothing consulted.
//!
//! On an 18-corpus L3-only board it is -82,653 bytes (-0.360%); here it is
//! diluted across nine levels, only one of which is DFast.
//!
//! It also widened the adjudicated L3->L5 ladder tie from 0.1% to 0.2%: L3
//! goes 3,517,111 -> 3,514,780 on full osdb while L5 stays at 3,519,696, the
//! exact value already recorded -- the cheaper level gaining again, not Greedy
//! losing. See `higher_level_never_larger_osdb`.
//!//! The source-sized-hash move is the only entry here that changes GOLD while
//! the TOTAL stays byte-for-byte identical (59,685,682 both sides). It sizes
//! the hash from the source rather than the window for BtLazy2 and above, so
//! individual frames shift while the sum does not -- measured +0 bytes at
//! L16/L19/L22 at every cap, +144 at L13/256K. What it buys is MEMORY: the
//! table allocation falls 25% and peak RSS 4.9-12.5%.
//!//! bext also introduced two LADDER INVERSIONS by making L3 beat L5 outright --
//! dickens +1.020%, osdb +0.074%. Both are the cheaper level GAINING a
//! capability, not the dearer one losing it; `higher_level_never_larger_osdb`
//! records the adjudication and keeps a ceiling on L5 so the exception cannot
//! mask a real Greedy regression.
const IDS: &[&str] = &[
    "zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray",
];
const LEVELS: &[i32] = &[1, 2, 3, 5, 7, 9, 12, 15, 19];
fn mix(a: &mut u64, b: &[u8]) {
    for &x in b {
        *a = (*a ^ u64::from(x)).wrapping_mul(0x100_0000_01B3);
    }
}
fn main() {
    let cap: usize = std::env::var("BG_CAP").ok().and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
    }).collect();
    let mut gold = 0xCBF2_9CE4_8422_2325u64;
    let mut total = 0usize;
    println!("BYTE-IDENTITY GATE  cap={cap} corpora={} levels={:?}\n", srcs.len(), LEVELS);
    {
        // STRATEGY COVERAGE, reported but NOT enforced -- deliberately.
        //
        // This list resolves to Fast, DFast, Greedy, Lazy, Lazy2, BtLazy2 and
        // BtUltra2, but NOT BtOpt: L12 is Lazy2 and L15 is BtLazy2, so the
        // first BtOpt level (16) is never compressed here. `find_opt` itself
        // is reached at L19, but through the BtUltra2 arm -- a defect confined
        // to the non-ultra BtOpt path would not move GOLD.
        //
        // NOT fixed by adding L16, because GOLD is this campaign's identity
        // anchor and its history is a curated record; widening the input set
        // moves it for a reason unrelated to any bitstream change, which would
        // corrupt exactly the signal the anchor exists to carry. `determall.rs`
        // (L16 included) and `simdparity.rs` both cover BtOpt today, so the
        // strategy is not unguarded -- only this gate does not see it. Whoever
        // next moves GOLD deliberately should fold L16 in at the same time and
        // record both reasons in the history block above.
        let mut seen: Vec<String> = LEVELS
            .iter()
            .filter_map(|&l| rusty_zstd::compression_params(l, None).ok())
            .map(|p| format!("{:?}", p.strategy))
            .collect();
        seen.sort();
        seen.dedup();
        const ALL: &[&str] = &["Fast", "DFast", "Greedy", "Lazy", "Lazy2", "BtLazy2",
                               "BtOpt", "BtUltra", "BtUltra2"];
        let missing: Vec<&str> = ALL
            .iter()
            .copied()
            .filter(|a| !seen.iter().any(|x| x == a))
            .collect();
        println!("strategies in GOLD: {}", seen.join(", "));
        if !missing.is_empty() {
            println!("NOT in GOLD: {} -- a bitstream change confined to those",
                     missing.join(", "));
            println!("             finders will NOT move this number.");
        }
    }
    print!("{:<14}", "corpus");
    for l in LEVELS { print!("{:>10}", format!("L{l}")); }
    println!();
    for (id, s) in &srcs {
        print!("{id:<14}");
        for &l in LEVELS {
            let z = rusty_zstd::compress(s, l).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("decompress")[..], &s[..], "{id} L{l}");
            mix(&mut gold, &z);
            total += z.len();
            print!("{:>10}", z.len());
        }
        println!();
    }
    println!("\ntotal compressed bytes {total}");
    println!("GOLD {gold:016X}");
}
