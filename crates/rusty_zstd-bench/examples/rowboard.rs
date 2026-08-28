//! E1 BOARD: the row match finder against the hash-chain walk it replaces.
//!
//! The row finder is **bitstream-CHANGING**, so byte-identity is the wrong
//! gate and `bytegate.rs` cannot judge it. Its gate is this board:
//!
//!   1. **Round-trip on every cell.** Non-negotiable -- a finder that changes
//!      the bitstream must still produce a frame that decodes to the source.
//!   2. **Compressed size per corpus.** Deterministic, one run, immune to the
//!      10-33% null arm this box carries. A row holds the last 16 positions
//!      for its bucket where the chain held all of them linked, so the row
//!      finder sees FEWER candidates and is expected to cost some ratio. This
//!      board prices that.
//!   3. **The deterministic work counts.** `WALK_EXAM` (chain candidates, each
//!      behind its own dependent load) against `ROW_EXAM` / `ROW_LOADS`. The
//!      whole claim is one dependent load per ROW instead of per CANDIDATE, so
//!      `ROW_LOADS` vs `WALK_EXAM` IS the claim, stated as a number.
//!
//! No clock. On this box a clock cannot resolve anything under ~30%, and the
//! ratio question is deterministic anyway.
//!
//! Requires --features profile for the census columns.
const IDS: &[&str] = &[
    "dickens", "mozilla", "samba", "webster", "xml", "x-ray", "osdb", "reymont",
    "nci", "sao", "mr", "ooffice",
];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}

fn main() {
    let levels: Vec<i32> = match std::env::args().nth(1) {
        Some(s) => s.split(',').map(|v| v.parse().unwrap()).collect(),
        None => vec![7, 9, 12],
    };
    let cap = 8usize << 20;
    println!("E1 ROW MATCH FINDER BOARD -- deterministic, no clock\n");
    for lvl in &levels {
        println!("## L{lvl}\n");
        println!(
            "| corpus | chain size | row size | ratio | chain loads | row loads | row cands | loads saved |"
        );
        println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
        let (mut ca, mut ra) = (0u64, 0u64);
        let (mut cl, mut rl, mut rc) = (0u64, 0u64, 0u64);
        let mut fails = 0usize;
        for id in IDS {
            let Some(f) = load(id) else { continue };
            let src = &f[..f.len().min(cap)];

            rusty_zstd::set_row_arm(false);
            let _ = rusty_zstd::take_walk_census();
            let a = rusty_zstd::compress(src, *lvl).unwrap();
            let (chain_loads, _) = rusty_zstd::take_walk_census();
            if rusty_zstd::decompress(&a).unwrap() != src {
                println!("| {id} | ROUND-TRIP FAILED (chain arm) |");
                fails += 1;
                continue;
            }

            rusty_zstd::set_row_arm(true);
            let _ = rusty_zstd::take_row_census();
            let b = rusty_zstd::compress(src, *lvl).unwrap();
            let (row_cands, row_loads) = rusty_zstd::take_row_census();
            if rusty_zstd::decompress(&b).unwrap() != src {
                println!("| {id} | **ROUND-TRIP FAILED (row arm)** |");
                fails += 1;
                continue;
            }
            rusty_zstd::set_row_arm(false);

            let ratio = b.len() as f64 / a.len() as f64;
            let saved = if row_loads > 0 {
                chain_loads as f64 / row_loads as f64
            } else {
                0.0
            };
            println!(
                "| {id} | {} | {} | {ratio:.4} | {chain_loads} | {row_loads} | {row_cands} | {saved:.2}x |",
                a.len(),
                b.len()
            );
            ca += a.len() as u64;
            ra += b.len() as u64;
            cl += chain_loads;
            rl += row_loads;
            rc += row_cands;
        }
        let ratio = ra as f64 / ca as f64;
        let saved = if rl > 0 { cl as f64 / rl as f64 } else { 0.0 };
        println!(
            "\n**L{lvl}: size {ratio:.4}x | chain loads {cl} -> row loads {rl} = {saved:.2}x fewer dependent loads | row candidates {rc}**"
        );
        if fails > 0 {
            println!("\n**{fails} ROUND-TRIP FAILURES -- do not ship.**");
        }
        println!();
    }
    println!("A row finder that costs size must EARN it in speed, and this box");
    println!("cannot measure speed below ~30%. Read the load ratio as the claim");
    println!("and the size ratio as the price; the speed verdict needs a quiet box.");
}
