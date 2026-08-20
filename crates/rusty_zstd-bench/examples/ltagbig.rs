//! 1a array route priced on FULL-LENGTH frames (>= 16 MiB, where the packed
//! form is refused and the long table previously ran unfiltered).
//!
//! A = `set_long_tag_arm(false)` == shipping behavior before the array route
//! B = default ON (ltags byte array, second cache line per probe -- the
//!     T1-array trade the short table's big-frame board accepted at 33%)
//!
//! The consume-site residual columns are the deciding instrument (they see
//! the speculation path); counters are read out BETWEEN arms (the "32M"
//! contamination lesson). Requires `--features profile`.
const IDS: &[&str] = &["versions-16m", "jsonlog-16m", "text-32m", "zeros-32m", "incomp-32m",
    "mozilla", "webster", "nci", "samba"];
fn main() {
    #[cfg(not(feature = "profile"))]
    panic!("run with --features profile");
    #[cfg(feature = "profile")]
    {
        let (mut cells, mut same) = (0usize, 0usize);
        let (mut tne, mut trej, mut tbf0, mut tbf) = (0u64, 0u64, 0u64, 0u64);
        println!("corpus         lvl   bytes(A=off)  bytes(B=on)   nonempty   rejected  rej%   consume bytes-fail: off -> on");
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            assert!(f.len() >= 0x0100_0000, "{id}: board is for pack-refused frames");
            for lvl in [3, 4] {
                rusty_zstd::set_long_tag_arm(false);
                let _ = rusty_zstd::take_long_tag();
                let _ = rusty_zstd::take_long_tag_residual();
                let za = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                let (nea, reja, _fa) = rusty_zstd::take_long_tag();
                let (bf0, _w0, _a0) = rusty_zstd::take_long_tag_residual();
                assert_eq!((nea, reja), (0, 0), "{id} L{lvl}: arm OFF must count nothing");
                rusty_zstd::set_long_tag_arm(true);
                let zb = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                assert!(rusty_zstd::decompress(&zb).unwrap() == f, "{id} L{lvl} round-trip");
                let (ne, rej, fal) = rusty_zstd::take_long_tag();
                let (bf, _w, _acc) = rusty_zstd::take_long_tag_residual();
                assert_eq!(fal, 0, "{id} L{lvl}: the long tag hid {fal} real matches");
                cells += 1;
                if za == zb { same += 1; } else { println!("  MISMATCH {id} L{lvl}: {} vs {}", za.len(), zb.len()); }
                let pct = if ne == 0 { 0.0 } else { 100.0 * rej as f64 / ne as f64 };
                println!("{id:14} L{lvl}  {:12} {:12} {ne:10} {rej:10}  {pct:4.1}%   {bf0:10} -> {bf:8}",
                    za.len(), zb.len());
                tne += ne;
                trej += rej;
                tbf0 += bf0;
                tbf += bf;
            }
        }
        println!("BYTE-IDENTICAL across the arm: {same}/{cells} cells");
        println!("LOAD-SITE: {trej} of {tne} nonempty long probes rejected ({:.1}%)",
            if tne == 0 { 0.0 } else { 100.0 * trej as f64 / tne as f64 });
        println!("CONSUME-SITE (spec included): unfiltered bytes-fail {tbf0} -> {tbf} ({:.2}% remain)",
            if tbf0 == 0 { 0.0 } else { 100.0 * tbf as f64 / tbf0 as f64 });
        assert_eq!(same, cells, "the long tag must not move bytes");
    }
}
