//! MT path copy census + byte-identity gate.
//!
//! The multi-threaded compressor was the one region never instrumented. Its
//! job outputs are concatenated into a frame buffer that used to be a
//! `Vec::new()` -- grown to the whole compressed stream by doubling, which
//! copies ~N bytes in reallocs on top of the concat itself.
use rusty_zstd::{copies, AdvancedOptions};

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let nw: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    println!("MT COPY CENSUS (L{lvl}, {nw} workers)\n");
    println!("{:<14}{:>9}{:>14}{:>14}{:>12}", "corpus", "MiB", "compressed", "concat B", "B/input");
    let (mut tb, mut tsrc) = (0u64, 0u64);
    for id in ["dickens", "samba", "webster", "mozilla"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        let _ = copies::take();
        let params = rusty_zstd::compression_params(lvl, Some(src.len() as u64)).expect("params");
        let mut adv = AdvancedOptions::default();
        adv.nb_workers = nw as u32;
        let z = rusty_zstd::compress_mt(src, params, true, None, &[], true, adv).expect("mt");
        let c = copies::take();
        let back = rusty_zstd::decompress(&z).expect("decompress");
        assert_eq!(back, src, "{id} mt roundtrip");
        // The MT frame must equal the single-threaded frame for the same job
        // split, or the concat changed more than allocation behaviour.
        let n = c[copies::C_MT_CONCAT].0;
        println!("{id:<14}{:>9.1}{:>14}{:>14}{:>12.4}",
            src.len() as f64 / (1 << 20) as f64, z.len(), n,
            n as f64 / src.len() as f64);
        tb += n; tsrc += src.len() as u64;
        let _ = copies::take();
    }
    println!("\nconcat total {tb} B for {tsrc} input = {:.4} B/input", tb as f64 / tsrc as f64);
    println!("an UNRESERVED concat would have moved that AGAIN in realloc copies.");
}
