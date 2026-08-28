//! D9/D12 adjudication: count EXECUTED `BitRev::reload` calls over the corpus,
//! plus the X1/X2 section split, so the two arrangements can be compared
//! deterministically instead of on a clock.
fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = [
            ("generated", "jsonlog-16m"), ("generated", "smallmsg-8m"),
            ("generated", "versions-16m"), ("generated", "text-32m"),
            ("silesia", "dickens"), ("silesia", "mozilla"), ("silesia", "mr"),
            ("silesia", "nci"), ("silesia", "ooffice"), ("silesia", "osdb"),
            ("silesia", "reymont"), ("silesia", "samba"), ("silesia", "sao"),
            ("silesia", "webster"), ("silesia", "xml"), ("silesia", "x-ray"),
        ];
        let _ = rusty_zstd::take_reload_calls();
        let _ = rusty_zstd::take_reload_refills();
        let _ = rusty_zstd::take_x4_x1_calls();
        let _ = rusty_zstd::take_x2_stats();
        let mut bytes = 0u64;
        for lvl in [1i32, 3, 9, 19] {
            for (dir, id) in ids {
                let Ok(f) = std::fs::read(format!("corpora/data/{dir}/{id}")) else { continue };
                let s = &f[..f.len().min(4 << 20)];
                let c = rusty_zstd::compress_with(
                    s, rusty_zstd::CompressOptions { level: lvl, checksum: false },
                ).unwrap();
                // reset AFTER compression: we are counting DECODE reloads only
                let _ = rusty_zstd::take_reload_calls();
                let _ = rusty_zstd::take_reload_refills();
                let d = rusty_zstd::decompress(&c).unwrap();
                assert_eq!(d.len(), s.len());
                bytes += s.len() as u64;
                RELOADS.with(|r| r.set(r.get() + rusty_zstd::take_reload_calls()));
                REFILLS.with(|r| r.set(r.get() + rusty_zstd::take_reload_refills()));
            }
        }
        let reloads = RELOADS.with(|r| r.get());
        let refills = REFILLS.with(|r| r.get());
        let x1 = rusty_zstd::take_x4_x1_calls();
        println!("decode reloads = {reloads}");
        println!("bytes decoded  = {bytes}");
        println!("reloads/MiB    = {:.0}", reloads as f64 / (bytes as f64 / 1048576.0));
        println!("refills        = {refills}  ({:.1}% of calls)",
            refills as f64 * 100.0 / reloads as f64);
        println!("early-outs     = {}", reloads - refills);
        println!("x1 sections    = {x1}");
    }
    #[cfg(not(feature = "profile"))]
    println!("needs --features profile");
}

#[cfg(feature = "profile")]
thread_local!(static RELOADS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) });
#[cfg(feature = "profile")]
thread_local!(static REFILLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) });
