//! How much of zeros-32m decompress is the xxh64 content checksum?
use std::time::Instant;

fn best<F: FnMut()>(n: u32, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = Instant::now();
        f();
        b = b.min(t.elapsed().as_secs_f64());
    }
    b
}

fn main() {
    let n_bytes: usize = 32 << 20;
    let src = vec![0u8; n_bytes];
    let n = 15;
    let mbps = |s: f64| (n_bytes as f64) / s / 1e6;

    for lvl in [1, 3] {
        let on = rusty_zstd::compress_with(
            &src,
            rusty_zstd::CompressOptions { level: lvl, checksum: true, ..Default::default() },
        )
        .expect("compress ck-on");
        let off = rusty_zstd::compress_with(
            &src,
            rusty_zstd::CompressOptions { level: lvl, checksum: false, ..Default::default() },
        )
        .expect("compress ck-off");

        let mut d = Vec::with_capacity(n_bytes);
        rusty_zstd::decompress_into(&mut d, &on).unwrap();
        let a = best(n, || { d.clear(); rusty_zstd::decompress_into(&mut d, &on).unwrap(); });
        let b = best(n, || { d.clear(); rusty_zstd::decompress_into(&mut d, &off).unwrap(); });

        println!("L{lvl}  checksum ON {:8.1} MB/s ({:.4} s) | OFF {:8.1} MB/s ({:.4} s)",
                 mbps(a), a, mbps(b), b);
        println!("      checksum costs {:.4} s = {:.0}% of the ON time", a - b, (a - b) / a * 100.0);
    }
}
