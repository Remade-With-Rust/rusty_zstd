//! Where do the bt walk's ITERATIONS actually live?
//!
//! Mean depth is 8.6 against a budget of 128-512, and only 2.6% of walks reach
//! the cap -- so the cap governs a tiny minority of walks. The question that
//! decides whether Gate 14 has a lever left: do those few walks own a LARGE
//! share of the total work?
//!
//! Iterations are a deterministic counter, so this needs no clock. Sweeping the
//! depth arm and watching iterations respond measures the concentration directly:
//! a corpus whose iterations collapse when the cap drops is cap-bound; one whose
//! iterations barely move is not.
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(512 << 10);
    println!("GATE 14 TAIL @ L{lvl} — iterations vs depth (deterministic), cap {} KiB", cap>>10);
    println!("{:<13} {:>10} {:>12} {:>8} | {:>10} {:>10} {:>10} | {:>9}",
        "corpus", "walks", "iters d=0", "full%", "d=-1", "d=-2", "d=-3", "size d=-2");
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut base_it = 0u64; let mut base_sz = 0usize; let mut w0 = 0u64; let mut f0 = 0u64;
        let mut row = String::new();
        for d in [0i32, -1, -2, -3] {
            rusty_zstd::set_search_log_delta(d);
            let _ = rusty_zstd::take_bt_iters();
            let z = rusty_zstd::compress(s, lvl).unwrap();
            let (w, it, full) = rusty_zstd::take_bt_iters();
            assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} d={d}: round-trip");
            if d == 0 {
                base_it = it; base_sz = z.len(); w0 = w; f0 = full;
                if w == 0 { break }
                row = format!("{:<13} {:>10} {:>12} {:>7.2}% |", id, w, it, full as f64/w as f64*100.0);
            } else {
                row += &format!(" {:>9.1}%", (it as f64/base_it.max(1) as f64 - 1.0)*100.0);
                if d == -2 {
                    let sd = (z.len() as f64/base_sz as f64 - 1.0)*100.0;
                    row += &format!(" | {:>8.3}%", sd);
                }
            }
        }
        rusty_zstd::set_search_log_delta(0);
        if w0 > 0 {
            // reorder: the size column was appended mid-row, so print as built
            let _ = f0;
            println!("{row}");
        }
    }
    println!("\n  iterations collapsing as the cap drops => the walk IS cap-bound there");
    println!("  iterations barely moving                 => the cap governs nothing");
}
