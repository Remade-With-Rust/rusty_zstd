//! SHORT-table consume-site census at L3/L4 -- the mirror of the ltag boards.
//! A = set_dfast_tag_arm(false): NO short filter (and no pack) -- the
//!     unfiltered baseline. Long-filter state is identical across arms in
//!     EFFECT (filters never change outcomes), so short-site traffic matches.
//! B = default ON.
//! Requires --features profile.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    #[cfg(not(feature = "profile"))]
    panic!("run with --features profile");
    #[cfg(feature = "profile")]
    {
        let (mut cells, mut same) = (0usize, 0usize);
        let (mut tbf0, mut tbf, mut tw, mut ta) = (0u64, 0u64, 0u64, 0u64);
        println!("corpus         lvl   bytes A/B equal   short bytes-fail: off -> on     win-fail   accepted");
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(6 << 20)];
            for lvl in [3, 4] {
                rusty_zstd::set_dfast_tag_arm(false);
                let _ = rusty_zstd::take_short_tag_residual();
                let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                let (bf0, _w0, _a0) = rusty_zstd::take_short_tag_residual();
                rusty_zstd::set_dfast_tag_arm(true);
                let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                assert!(rusty_zstd::decompress(&zb).unwrap() == s, "{id} L{lvl} round-trip");
                let (bf, w, a) = rusty_zstd::take_short_tag_residual();
                cells += 1;
                if za == zb { same += 1; } else { println!("  MISMATCH {id} L{lvl}"); }
                println!("{id:14} L{lvl}      {}        {bf0:10} -> {bf:9}   {w:9}  {a:9}", za == zb);
                tbf0 += bf0; tbf += bf; tw += w; ta += a;
            }
        }
        println!("BYTE-IDENTICAL: {same}/{cells}");
        println!("SHORT CONSUME-SITE: unfiltered bytes-fail {tbf0} -> tagged {tbf} ({:.1}% remain), win-fail {tw}, accepted {ta}",
            if tbf0 == 0 { 0.0 } else { 100.0 * tbf as f64 / tbf0 as f64 });
        assert_eq!(same, cells);
    }
}
