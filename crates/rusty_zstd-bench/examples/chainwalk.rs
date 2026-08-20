//! The chain-walk amputation defect, adjudicated. The walk broke on the first
//! byte-mismatching candidate (a hash collision), abandoning the rest of the
//! chain; C steps past it. The fix is byte-CHANGING (finds matches the
//! amputated walk missed), so it ships on this board, not on byte-identity.
//!
//! A = legacy break (shipping)   B = walk-continue (C parity)
//! Census: candidates examined + byte-mismatch steps (the added work).
//! Requires --features profile for the census columns.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let arg = std::env::args().nth(1);
    let levels: Vec<i32> = match arg.as_deref() {
        Some(s) => s.split(',').map(|v| v.parse().unwrap()).collect(),
        None => vec![5, 7, 9, 12],
    };
    for lvl in levels {
        let (mut ta, mut tb) = (0u64, 0u64);
        let (mut tex_a, mut tex_b, mut tmiss_b) = (0u64, 0u64, 0u64);
        let mut worst = (0.0f64, "-");
        println!("L{lvl}:  corpus         off-bytes   on-bytes    delta%     exam off -> on (bytemiss)");
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(6 << 20)];
            rusty_zstd::set_walk_cont_arm(false);
            #[cfg(feature = "profile")]
            let _ = rusty_zstd::take_walk_census();
            let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            #[cfg(feature = "profile")]
            let (exa, _ma) = rusty_zstd::take_walk_census();
            #[cfg(not(feature = "profile"))]
            let exa = 0u64;
            rusty_zstd::set_walk_cont_arm(true);
            let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            assert!(rusty_zstd::decompress(&zb).unwrap() == s, "{id} L{lvl} round-trip");
            #[cfg(feature = "profile")]
            let (exb, mb) = rusty_zstd::take_walk_census();
            #[cfg(not(feature = "profile"))]
            let (exb, mb) = (0u64, 0u64);
            let d = 100.0 * (zb.len() as f64 - za.len() as f64) / za.len() as f64;
            if d > worst.0 { worst = (d, id); }
            println!("     {id:14} {:10} {:10}  {d:+7.4}%   {exa:11} -> {exb:11} ({mb})", za.len(), zb.len());
            ta += za.len() as u64;
            tb += zb.len() as u64;
            tex_a += exa;
            tex_b += exb;
            tmiss_b += mb;
        }
        println!("L{lvl} TOTAL: {ta} -> {tb} ({:+.4}%)  exams {tex_a} -> {tex_b} ({:+.1}%), bytemiss steps {tmiss_b}, worst {} {:+.4}%",
            100.0*(tb as f64 - ta as f64)/ta as f64,
            if tex_a==0 {0.0} else {100.0*(tex_b as f64 - tex_a as f64)/tex_a as f64},
            worst.1, worst.0);
    }
}
