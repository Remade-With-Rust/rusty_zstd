//! Re-adjudicate the two candidates I wrongly called INERT.
//!
//! `tag_min` gates the packed rejection tag and `dfast_spec_min` gates DFast's
//! speculated loads. BOTH are byte-identical by construction -- the tag cannot
//! hide a match, and a speculation is either consumed or discarded. So a SIZE
//! sweep measuring 0.0000% is not evidence of inertness; it is evidence I
//! measured the wrong axis. Their axis is WORK.
const IDS: &[&str] = &["dickens","samba","mozilla","x-ray","sao","osdb"];
fn main() {
    let cap = 4 << 20;
    println!("tag_min (L1) -- axis: candidate loads AVOIDED by the tag filter");
    println!("   {:<8} {:>14} {:>14} {:>10}", "tag_min", "probes", "loads avoided", "avoided%");
    for v in [0.0f32, 0.25, 0.5, 0.75, 0.9, 1.0] {
        std::env::set_var("RZSTD_TAG_T", v.to_string());
        let (mut p, mut r) = (0u64, 0u64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            rusty_zstd::prof_reset();
            let _ = rusty_zstd::take_tag_rejects();
            let _ = rusty_zstd::compress(s, 1).unwrap();
            let c = rusty_zstd::prof_encode_counts();
            let (_fals, rej) = rusty_zstd::take_tag_rejects();
            p += c.hash_probes; r += rej;
        }
        println!("   {v:<8} {p:>14} {r:>14} {:>9.1}%", r as f64 / p.max(1) as f64 * 100.0);
    }
    println!("\ndfast_spec_min (L3) -- axis: speculated loads MADE vs CONSUMED");
    println!("   {:<14} {:>14} {:>14} {:>10}", "dfast_spec_min", "spec made", "spec used", "wasted%");
    for v in [0.0f32, 0.25, 0.5, 0.75, 0.9, 1.0] {
        rusty_zstd::set_dfast_spec_min_arm(v);
        let (mut md, mut us) = (0u64, 0u64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            let _ = rusty_zstd::take_dfast_spec();
            let _ = rusty_zstd::compress(s, 3).unwrap();
            let (m, u) = rusty_zstd::take_dfast_spec();
            md += m; us += u;
        }
        println!("   {v:<14} {md:>14} {us:>14} {:>9.1}%", (1.0 - us as f64 / md.max(1) as f64) * 100.0);
    }
}
