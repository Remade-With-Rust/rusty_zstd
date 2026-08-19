//! What does one fresh per-block buffer actually COST?
//!
//! Gate 6 keeps finding the same shape: a buffer sized from `block_len`, built
//! fresh every block, landing just over 128 KiB. Before restructuring
//! `huffman.rs` to remove 65 more of them per frame, price ONE of them.
//!
//! This is a pure microbenchmark -- no codec -- so it is immune to whatever
//! else is being edited, and it runs enough iterations that the per-op cost
//! escapes the +-24% noise floor that defeats a per-frame measurement.
//!
//! Two arms, identical WORK, different allocation strategy:
//!   fresh -- Vec::with_capacity(sz) per iteration, filled, dropped
//!   reuse -- one Vec, cleared and refilled
//!
//! The size sweep is the point: if there is a cliff at the large-allocation
//! threshold, the fix generalises to every buffer sized from `block_len`.
use std::time::Instant;

fn main() {
    let iters: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let src = vec![0x5au8; 1 << 20];
    println!("cost of a FRESH per-block buffer vs a REUSED one, {iters} iterations each\n");
    println!("{:>10} {:>12} {:>12} {:>10} {:>14}", "size", "fresh ns/op", "reuse ns/op", "delta%", "fresh-reuse ns");
    println!("{}", "-".repeat(62));

    let sizes = [131_136, 138_000, 512 << 10, 1 << 20, 2 << 20, 4 << 20];
    for sz in sizes {
        let mut best_fresh = f64::MAX;
        let mut best_reuse = f64::MAX;
        // ABBA, three passes, best-of -- the same discipline the gates use
        for pass in 0..3 {
            for arm in [pass % 2, 1 - pass % 2] {
                let t = Instant::now();
                if arm == 0 {
                    for _ in 0..iters {
                        let mut v: Vec<u8> = Vec::with_capacity(sz);
                        v.extend_from_slice(&src[..sz]);
                        std::hint::black_box(&v);
                    }
                } else {
                    let mut v: Vec<u8> = Vec::with_capacity(sz);
                    for _ in 0..iters {
                        v.clear();
                        v.extend_from_slice(&src[..sz]);
                        std::hint::black_box(&v);
                    }
                }
                let ns = t.elapsed().as_secs_f64() * 1e9 / iters as f64;
                if arm == 0 { if ns < best_fresh { best_fresh = ns } }
                else if ns < best_reuse { best_reuse = ns }
            }
        }
        let mark = if sz >= (128 << 10) { " <- at/over 128 KiB" } else { "" };
        println!("{:>9}K {:>12.1} {:>12.1} {:>9.1}% {:>14.1}{}",
            sz >> 10, best_fresh, best_reuse, (best_fresh / best_reuse - 1.0) * 100.0,
            best_fresh - best_reuse, mark);
    }
    println!("\n  The `fresh - reuse` column is what one avoided per-block allocation buys.");
    println!("  Multiply by the per-frame count to price a restructure.");
}
