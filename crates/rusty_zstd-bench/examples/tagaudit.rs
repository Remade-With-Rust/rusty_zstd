//! Tag-system audit receipt.
//!
//! The one way the tag machinery can corrupt OUTPUT is a false reject: the
//! filter hiding a candidate that would have matched. At the Fast levels the
//! `TAG_FALSE_REJECT` counter re-probes every rejected raw slot and counts
//! exactly that, so it must be ZERO. (At L3 the same counter is reused by
//! DFast's T1 ledger to mean plain rejections -- the filter's WINS -- so the
//! zero-assertion applies to L1/L2 only; L3 is reported, not asserted.)
//!
//! Requires `--features profile`.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    #[cfg(not(feature = "profile"))]
    panic!("run with --features profile");
    #[cfg(feature = "profile")]
    {
        let _ = rusty_zstd::take_tag_rejects();
        for lvl in [1i32, 2, 3] {
            let (mut fr, mut tot, mut n) = (0u64, 0u64, 0usize);
            for id in IDS {
                let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
                let s = &f[..f.len().min(6 << 20)];
                let z = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} L{lvl} round-trip");
                let (f_, t_) = rusty_zstd::take_tag_rejects();
                fr += f_;
                tot += t_;
                n += 1;
            }
            if lvl <= 2 {
                println!("L{lvl}: {n} corpora, tag rejects {tot}, FALSE rejects {fr}  (must be 0)");
                assert_eq!(fr, 0, "L{lvl}: the tag filter hid {fr} real matches");
            } else {
                println!("L{lvl}: {n} corpora, nonempty short slots {tot}, rejections {fr}  (DFast T1 ledger semantics)");
            }
        }
        println!("TAG AUDIT PASS: no false rejects at the Fast levels");
    }
}
