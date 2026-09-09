//! Byte gate for the `--long` (LDM) path, which `bytegate` never exercises:
//! compress a few corpora with LDM enabled at L3/L9/L19 and fold every output
//! byte into one FNV-1a hash. Run before and after an LDM change; the GOLD
//! line must not move. Under `--features profile` it also prints BRICK 18's
//! deterministic counters: candidates that passed the window tests (the
//! population the old per-candidate `memcmp` ran on) and those the 8-byte
//! head let through to the count.
use rusty_zstd::{compress_with_advanced, compression_params, AdvancedOptions, LdmParams};

const IDS: &[&str] = &["dickens", "mozilla", "webster", "xml", "samba"];

fn main() {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut total = 0usize;
    println!(
        "{:<10} {:>4} {:>10} {:>10}   ldm candidates -> counted",
        "corpus", "L", "in", "out"
    );
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
            continue;
        };
        let src = &full[..full.len().min(16 << 20)];
        for lvl in [3i32, 9, 19] {
            let params = compression_params(lvl, Some(src.len() as u64)).unwrap();
            #[cfg(feature = "profile")]
            let _ = rusty_zstd::take_ldm_stats();
            let out = compress_with_advanced(
                src,
                params,
                true,
                None,
                &[],
                true,
                AdvancedOptions {
                    ldm: LdmParams::enabled(),
                    ..AdvancedOptions::default()
                },
            )
            .unwrap();
            for &b in &out {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            total += out.len();
            #[cfg(feature = "profile")]
            {
                let (c, k) = rusty_zstd::take_ldm_stats();
                println!(
                    "{:<10} {:>4} {:>10} {:>10}   {:>10} -> {:<10} ({:.1}% counted)",
                    id,
                    lvl,
                    src.len(),
                    out.len(),
                    c,
                    k,
                    100.0 * k as f64 / c.max(1) as f64
                );
            }
            #[cfg(not(feature = "profile"))]
            println!("{:<10} {:>4} {:>10} {:>10}", id, lvl, src.len(), out.len());
        }
    }
    println!("total compressed bytes {total}");
    println!("LDM GOLD {h:016X}");
}
