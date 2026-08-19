//! T1 ledger: at L3, how often does the tag actually reject?
//! Here TAG_REJECT_TOTAL = short-table probes that found a NON-EMPTY slot, and
//! TAG_FALSE_REJECT = those the tag then rejected, i.e. candidate loads AVOIDED.
//! (Names inherited from the Fast-path audit; read them as denominator/numerator.)
const IDS: &[&str] = &["x-ray","sao","ooffice","smallmsg-8m","mozilla","osdb",
                       "dickens","samba","nci","webster","mr","jsonlog-16m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = 8 << 20;
    rusty_zstd::set_dfast_tag_arm(true);
    println!("T1 REJECT RATE @ L{lvl}\n");
    println!("{:<13} {:>12} {:>13} {:>12} {:>9}", "corpus", "probes", "slots non-empty", "rejected", "reject%");
    println!("{}", "-".repeat(64));
    let (mut tp, mut td, mut tr) = (0u64, 0u64, 0u64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_tag_rejects();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let c = rusty_zstd::prof_encode_counts();
        let (rej, denom) = rusty_zstd::take_tag_rejects();
        tp += c.hash_probes; td += denom; tr += rej;
        println!("{:<13} {:>12} {:>13} {:>12} {:>8.1}%",
            id, c.hash_probes, denom, rej, rej as f64 / denom.max(1) as f64 * 100.0);
    }
    println!("\n  TOTAL: {td} non-empty short slots, {tr} rejected without loading the candidate ({:.1}%)",
        tr as f64 / td.max(1) as f64 * 100.0);
    println!("  Against {tp} counted probes overall.");
}
