//! 1a: the LONG-table rejection tag, priced on the census A/B — per-corpus
//! buckets, byte-identity asserted per cell, and the FALSE-reject counter
//! asserted zero (a false reject is the only way this filter could move
//! bytes, and the proof says it cannot: the tag is the SHORT tag, a function
//! of the first 4 bytes, and every long acceptance verifies >= 4 leading
//! bytes).
//!
//! A = `set_long_tag_arm(false)` == shipping behavior before 1a
//! B = default ON
//!
//! Requires `--features profile`.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    #[cfg(not(feature = "profile"))]
    panic!("run with --features profile");
    #[cfg(feature = "profile")]
    {
        let (mut cells, mut same) = (0usize, 0usize);
        let (mut tne, mut trej) = (0u64, 0u64);
        let (mut tsf, mut tsw, mut tsa) = (0u64, 0u64, 0u64);
        let mut tbf0 = 0u64;
        println!("corpus         lvl   bytes(A=off)  bytes(B=on)   nonempty   rejected  rej%");
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(6 << 20)];
            for lvl in [3, 4] {
                rusty_zstd::set_long_tag_arm(false);
                let _ = rusty_zstd::take_long_tag();
                let _ = rusty_zstd::take_long_tag_residual();
                let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                let (nea, reja, _fa) = rusty_zstd::take_long_tag();
                // COUNTER HYGIENE (the 32M lesson): the residual statics ran
                // during the arm-OFF pass too; read them out as the UNFILTERED
                // baseline, or they contaminate the arm-B residual 4:1.
                let (bf0, _w0, _a0) = rusty_zstd::take_long_tag_residual();
                assert_eq!((nea, reja), (0, 0), "{id} L{lvl}: arm OFF must count nothing");
                rusty_zstd::set_long_tag_arm(true);
                let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                assert!(rusty_zstd::decompress(&zb).unwrap() == s, "{id} L{lvl} round-trip");
                let (ne, rej, fal) = rusty_zstd::take_long_tag();
                let (sf, sw, sa) = rusty_zstd::take_long_tag_residual();
                assert_eq!(fal, 0, "{id} L{lvl}: the long tag hid {fal} real matches");
                cells += 1;
                if za == zb { same += 1; } else { println!("  MISMATCH {id} L{lvl}: {} vs {}", za.len(), zb.len()); }
                let pct = if ne == 0 { 0.0 } else { 100.0 * rej as f64 / ne as f64 };
                let spct = if sf + sw + sa == 0 { 0.0 } else { 100.0 * sf as f64 / (sf + sw + sa) as f64 };
                println!("{id:14} L{lvl}  {:12} {:12} {ne:10} {rej:10}  {pct:4.1}%  bytes-fail: unfiltered {bf0:9} -> tagged {sf:7} ({spct:4.1}%)  win-fail {sw:7}", za.len(), zb.len());
                tne += ne;
                trej += rej;
                tsf += sf;
                tsw += sw;
                tsa += sa;
                tbf0 += bf0;
            }
        }
        println!("BYTE-IDENTICAL across the arm: {same}/{cells} cells");
        println!("TOTAL: {trej} long candidate loads avoided of {tne} nonempty probes ({:.1}%), FALSE 0",
            if tne == 0 { 0.0 } else { 100.0 * trej as f64 / tne as f64 });
        println!("CONSUME-SITE LEDGER (spec path included): unfiltered bytes-fail {tbf0} -> tagged {tsf} ({:.2}% remain, the 8-bit collision floor), window-fail {tsw}, accepted {tsa}",
            if tbf0 == 0 { 0.0 } else { 100.0 * tsf as f64 / tbf0 as f64 });
        assert_eq!(same, cells, "the long tag must not move bytes");
    }
}
