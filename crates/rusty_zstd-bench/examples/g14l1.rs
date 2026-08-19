//! GATE 14 @ L1 — chain-walk depth `1 << search_log`.
//!
//! STEP 1, the dead check, run across all four campaign levels so the SCOPE of
//! the gate is measured rather than asserted. `find_fast` contains zero
//! references to `search_attempts` or `search_log`; this proves it on the output.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    println!("GATE 14 DEAD CHECK — set_search_log_delta, cap {} KiB", cap>>10);
    println!("{:>5} {:>10} {:>10} {:>10} {:>10} | {}", "lvl", "d=-2", "d=-1", "d=+1", "d=+2", "verdict");
    for lvl in [1i32, 3, 5, 13, 19, 22] {
        let mut moved = [0usize; 4];
        let mut n = 0;
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            rusty_zstd::set_search_log_delta(0);
            let base = rusty_zstd::compress(s, lvl).unwrap();
            for (i, d) in [-2i32, -1, 1, 2].iter().enumerate() {
                rusty_zstd::set_search_log_delta(*d);
                let z = rusty_zstd::compress(s, lvl).unwrap();
                assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} L{lvl} d={d}: round-trip");
                if z != base { moved[i] += 1; }
            }
            rusty_zstd::set_search_log_delta(0);
            n += 1;
        }
        let total: usize = moved.iter().sum();
        println!("{:>5} {:>9}/{} {:>9}/{} {:>9}/{} {:>9}/{} | {}",
            lvl, moved[0], n, moved[1], n, moved[2], n, moved[3], n,
            if total == 0 { "DEAD -- no arm reaches the output" } else { "LIVE" });
    }
}
