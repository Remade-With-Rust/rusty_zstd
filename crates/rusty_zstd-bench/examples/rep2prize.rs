//! THE SECOND REPCODE'S PRIZE, measured before it is built.
//!
//!   cargo run --release -p rusty_zstd-bench --features profile --example rep2prize -- [levels] [ids]
//!
//! C's fast and dfast loops test `read32(ip) == read32(ip - offset_2)` right
//! after storing a match and emit zero-literal repcode matches in a loop,
//! swapping the two offsets each time. This crate's Fast and DFast finders
//! have no such test. `Rep2Probe` (profile only, no behaviour change) asks C's
//! question after every sequence those finders emit, with the exact offset
//! history, and this prints the answer per corpus and level:
//!
//!   seqs     sequences emitted
//!   fire     how often C's test would fire (and as a share of sequences)
//!   bytes    the bytes those zero-literal matches would cover (share of input)
//!   chain    further matches / bytes when C's loop is followed (swap, re-test)
//!   already  fires our own parse takes anyway: its next sequence IS a
//!            zero-literal match at that offset (and the bytes of overlap)
//!   new      fire - already: the opportunities nothing captures today
//!
//! Counts, not clocks; and not yet a SIZE -- a covered byte may already be
//! covered by a hash match at a full offset, which is exactly the trade the
//! repcode wins. The size verdict needs the arm.
#[cfg(feature = "profile")]
const DEFAULT_IDS: &[&str] = &["dickens", "mozilla", "samba", "nci", "xml", "x-ray"];

fn main() {
    #[cfg(not(feature = "profile"))]
    {
        println!("needs --features profile");
    }
    #[cfg(feature = "profile")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let levels: Vec<i32> = args.iter().filter_map(|a| a.parse().ok()).collect();
        let levels = if levels.is_empty() { vec![1, 2, 3, 4] } else { levels };
        let ids: Vec<String> = args.iter().filter(|a| a.parse::<i32>().is_err()).cloned().collect();
        let ids: Vec<String> =
            if ids.is_empty() { DEFAULT_IDS.iter().map(|s| (*s).to_string()).collect() } else { ids };
        for lvl in levels {
            println!("\n=== L{lvl} -- the rep2 test after every emitted sequence ===");
            println!(
                "{:<9} {:>9} {:>9} {:>6} {:>10} {:>6} {:>8} {:>9} {:>8} {:>9} {:>8} {:>6}  {:>10}",
                "corpus", "seqs", "fire", "%seq", "bytes", "%in", "chain", "chainB", "already", "alreadyB", "new", "%seq",
                "csize"
            );
            for id in &ids {
                let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
                else {
                    continue;
                };
                let _ = rusty_zstd::take_rep2_prize();
                let z = rusty_zstd::compress(&f, lvl).expect("compress");
                let p = rusty_zstd::take_rep2_prize();
                let seqs = p[0].max(1) as f64;
                let new = p[1] - p[5].min(p[1]);
                println!(
                    "{:<9} {:>9} {:>9} {:>5.1}% {:>10} {:>5.2}% {:>8} {:>9} {:>8} {:>9} {:>8} {:>5.1}%  {:>10}",
                    id,
                    p[0],
                    p[1],
                    100.0 * p[1] as f64 / seqs,
                    p[2],
                    100.0 * p[2] as f64 / f.len() as f64,
                    p[3],
                    p[4],
                    p[5],
                    p[6],
                    new,
                    100.0 * new as f64 / seqs,
                    z.len(),
                );
            }
        }
    }
}
