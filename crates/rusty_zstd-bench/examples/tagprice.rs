//! What does the packed rejection tag ALREADY buy on the Fast ladder?
//!
//!   cargo run --release --features rusty_zstd/profile -p rusty_zstd-bench --example tagprice -- 1
//!
//! `find_fast_impl` is generic over `const PACKED: bool`. With it on,
//! `load_fast::<true>` compares a tag byte held IN the table entry and rejects a
//! bad candidate WITHOUT loading the candidate's bytes -- i.e. without the
//! random-access cache miss that dominates a missing probe.
//!
//! `find_dfast_impl` passes `false`. It never uses the mechanism.
//!
//! So: measure what the tag saves where it IS used, to price carrying it where
//! it is not. `rejects` = candidate loads avoided. `false` = rejects that would
//! actually have matched -- the ratio cost of the tag, paid in missed matches.
const IDS: &[&str] = &["x-ray","sao","ooffice","smallmsg-8m","mozilla","osdb",
                       "dickens","samba","nci","webster","mr","jsonlog-16m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = 8 << 20;
    println!("TAG PRICE @ L{lvl} -- what the packed tag saves where it is already used\n");
    println!("{:<13} {:>12} {:>12} {:>9} {:>10} {:>9}",
        "corpus", "probes", "loads saved", "saved%", "false rej", "false%");
    println!("{}", "-".repeat(70));
    let (mut tp, mut tr, mut tf) = (0u64, 0u64, 0u64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_tag_rejects();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let c = rusty_zstd::prof_encode_counts();
        let (fals, rej) = rusty_zstd::take_tag_rejects();
        let p = c.hash_probes;
        tp += p; tr += rej; tf += fals;
        println!("{:<13} {:>12} {:>12} {:>8.1}% {:>10} {:>8.2}%",
            id, p, rej, rej as f64 / p.max(1) as f64 * 100.0, fals,
            fals as f64 / rej.max(1) as f64 * 100.0);
    }
    println!("\n  TOTAL: {tp} probes, {tr} candidate loads avoided ({:.1}%), {tf} of those were false ({:.2}%)",
        tr as f64 / tp.max(1) as f64 * 100.0, tf as f64 / tr.max(1) as f64 * 100.0);
    println!("\n  Every avoided load is a random access into a table far larger than L2.");
    println!("  `find_dfast_impl` passes PACKED=false and therefore avoids NONE of them.");
}
