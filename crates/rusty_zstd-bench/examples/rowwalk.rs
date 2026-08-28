//! W2's RECEIPT -- the row walk's slot-visit cost, old form vs new, priced
//! from the SAME masks in one run (no A/B build, no clock).
//!
//! The old `candidates` loop stepped k = 1..=ROW and re-tested the mask at the
//! top, so it visited every slot from the newest down to the LAST matching
//! one. The new bit-walk rotates the mask so the newest slot is the high bit
//! and consumes set bits directly: one iteration per candidate. Both costs are
//! computed from each probe's own mask, so this is a count, not a measurement.
//! Requires --features profile, and the row arm ON.
const IDS: &[&str] = &["dickens", "webster", "samba", "xml", "nci", "reymont", "osdb", "mozilla"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = 8 << 20;
    rusty_zstd::set_row_arm(true);
    let _ = rusty_zstd::take_row_walk();
    println!("ROW WALK CENSUS @ L{lvl} -- slot visits, old form vs new\n");
    println!("{:<10}{:>12}{:>14}{:>14}{:>10}", "corpus", "probes", "OLD visits", "NEW visits", "ratio");
    let (mut tp, mut to, mut tn) = (0u64, 0u64, 0u64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(cap)];
        let _ = rusty_zstd::take_row_walk();
        let z = rusty_zstd::compress(src, lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id}: round-trip");
        let [p, o, n] = rusty_zstd::take_row_walk();
        if p == 0 {
            continue;
        }
        println!(
            "{:<10}{:>12}{:>14}{:>14}{:>9.2}x",
            id, p, o, n, o as f64 / n.max(1) as f64
        );
        tp += p;
        to += o;
        tn += n;
    }
    rusty_zstd::set_row_arm(false);
    println!(
        "\n**TOTAL {} probes: OLD {} slot visits -> NEW {} = {:.2}x fewer**",
        tp, to, tn, to as f64 / tn.max(1) as f64
    );
    println!(
        "per probe: old {:.2} visits, new {:.2}",
        to as f64 / tp as f64,
        tn as f64 / tp as f64
    );
}
