//! Price the ONE primitive both finders share: count_match -> count_eq_len_ge8.
//! calls per scanned position, and the returned-length histogram that decides
//! whether a wider kernel could ever help.
const IDS: &[&str] = &["x-ray", "osdb", "jsonlog-16m", "smallmsg-8m", "ooffice", "sao",
                       "dickens", "samba", "nci", "webster", "mozilla", "mr"];
fn main() {
    #[cfg(feature = "profile")]
    {
        let cap: usize = 8 << 20;
        for lvl in [1i32, 3] {
            let p = rusty_zstd::compression_params(lvl, None).unwrap();
            let _ = rusty_zstd::take_eqlen_stats();
            let _ = rusty_zstd::take_mm();
            let (mut pos, mut bytes) = (0u64, 0f64);
            for id in IDS {
                let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
                let s = &f[..f.len().min(cap)];
                let _ = rusty_zstd::compress(s, lvl).unwrap();
                bytes += s.len() as f64;
            }
            pos += rusty_zstd::take_mm().0;
            let (calls, work, h) = rusty_zstd::take_eqlen_stats();
            let t: u64 = h.iter().sum::<u64>().max(1);
            println!("\n=== L{lvl} ({:?}) ===", p.strategy);
            println!("  positions scanned      {pos}");
            println!("  count_eq_len_ge8 calls {calls}   ({:.3} per position)", calls as f64 / pos.max(1) as f64);
            println!("  bytes compared         {work}   ({:.1} per call)", work as f64 / calls.max(1) as f64);
            println!("  board                  {:.1} MB", bytes / 1e6);
            println!("  returned-length histogram:");
            for (i, n) in ["<3","3-7","8-31","32-63","64-255","256+"].iter().zip(h.iter()) {
                println!("    {:<8} {:>12}  {:>5.1}%", i, n, *n as f64 / t as f64 * 100.0);
            }
        }
    }
    #[cfg(not(feature = "profile"))]
    println!("needs --features rusty_zstd/profile");
}
