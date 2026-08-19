//! WHERE DOES THE 2.36x GO? Stage attribution plus deterministic work counters.
//!
//!   cargo run --release --features rusty_zstd/profile -p rusty_zstd-bench --example hotspot -- 3
//!
//! The stage timers are distorted by their own instrumentation, so they are read
//! only as SHARES, and only to rank. The counters beside them are exact and
//! undistorted -- `probes/byte` in particular, which is the number that decides
//! whether we are slow because we do more work per position or because each
//! position costs more. Those are different defects with different fixes, and
//! the board cannot tell them apart.
use rusty_zstd::ProfStage;

const IDS: &[&str] = &["x-ray", "osdb", "jsonlog-16m", "smallmsg-8m", "ooffice", "sao",
                       "dickens", "samba", "nci", "webster", "mozilla", "mr"];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("HOTSPOT @ L{lvl} ({} MiB board)\n", cap >> 20);
    println!("{:<13} {:>7} {:>7} {:>7} {:>7} {:>7} | {:>9} {:>8} {:>8} {:>7}",
        "corpus", "find%", "entr%", "huff%", "fse%", "seqc%", "probes/B", "hit%", "seqs/KB", "lit%");
    println!("{}", "-".repeat(102));
    let mut agg: Vec<(f64, f64, f64, f64, f64, f64)> = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let c = rusty_zstd::prof_encode_counts();
        let ns = |st: ProfStage| rusty_zstd::prof_stage_ns(st) as f64;
        let tot = ns(ProfStage::EncodeTotal).max(1.0);
        let (find, ent) = (ns(ProfStage::EncodeMatchFind), ns(ProfStage::EncodeEntropy));
        let (huff, fse, sqc) = (ns(ProfStage::EncodeHuff), ns(ProfStage::EncodeFseSeq), ns(ProfStage::EncodeSeqCode));
        let n = s.len() as f64;
        let ppb = c.hash_probes as f64 / n;
        let hit = c.probe_hits as f64 / c.hash_probes.max(1) as f64 * 100.0;
        let spk = c.seqs as f64 / (n / 1024.0);
        let litp = c.lit_bytes as f64 / n * 100.0;
        agg.push((find / tot, ent / tot, huff / tot, fse / tot, sqc / tot, ppb));
        println!("{:<13} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% | {:>9.2} {:>7.1}% {:>8.1} {:>6.1}%",
            id, find / tot * 100.0, ent / tot * 100.0, huff / tot * 100.0,
            fse / tot * 100.0, sqc / tot * 100.0, ppb, hit, spk, litp);
    }
    let k = agg.len().max(1) as f64;
    let m = |f: fn(&(f64,f64,f64,f64,f64,f64)) -> f64| agg.iter().map(f).sum::<f64>() / k;
    println!("\n  mean shares: matchfind {:.1}%, entropy {:.1}% (huff {:.1}%, fse {:.1}%, seqcode {:.1}%)",
        m(|a| a.0)*100.0, m(|a| a.1)*100.0, m(|a| a.2)*100.0, m(|a| a.3)*100.0, m(|a| a.4)*100.0);
    println!("  mean probes per input byte: {:.2}", m(|a| a.5));
    println!("\n  A probes/byte near 1.0 means one search step per position -- the floor for");
    println!("  this algorithm. Well above it means we RE-search ground we already covered,");
    println!("  which is an algorithmic defect. Near it means each probe is simply too");
    println!("  expensive, which is a codegen defect. The fixes do not resemble each other.");
}
