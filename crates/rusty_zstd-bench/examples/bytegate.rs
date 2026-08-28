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
//! bext also introduced two LADDER INVERSIONS by making L3 beat L5 outright --
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
