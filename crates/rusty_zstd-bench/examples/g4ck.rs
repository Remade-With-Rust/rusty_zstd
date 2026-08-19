//! GATE 4 STEP 3 — streamed decode checksum vs the single final pass.
//! Correctness first: both arms must accept a good frame and REJECT a corrupt
//! one. Then the speed, on the side where the tax actually lands.
use rusty_zstd::{CompressOptions, DecompressOptions};
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","versions-16m","jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    println!("GATE 4 decode checksum: streamed vs final pass @ L{lvl}, cap {} KiB", cap>>10);
    println!("{:<13} {:>10} {:>10} {:>9} | {:>8} {:>8}", "corpus", "final ms", "stream ms", "delta%", "good", "corrupt");
    let (mut tf, mut ts) = (0.0f64, 0.0f64);
    let (mut okg, mut okc, mut n) = (0, 0, 0);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress_with(s, CompressOptions{level:lvl, checksum:true}).unwrap();
        // corrupt a payload byte, NOT the trailer, so the checksum must catch it
        let mut bad = z.clone();
        let m = bad.len()/2;
        bad[m] ^= 0x01;
        let mut buf = Vec::with_capacity(s.len());
        let mut good_ok = true; let mut corrupt_caught = true;
        let mut t = [0.0f64; 2];
        for (i, on) in [false, true].iter().enumerate() {
            rusty_zstd::set_ck_stream_arm(*on);
            let d = rusty_zstd::decompress(&z).unwrap();
            if d != s { good_ok = false }
            // a corrupt payload must be rejected -- by the checksum or earlier
            if rusty_zstd::decompress(&bad).map(|v| v == s).unwrap_or(false) { corrupt_caught = false }
            t[i] = best(7, || { buf.clear(); rusty_zstd::decompress_into(&mut buf, &z).unwrap() });
        }
        rusty_zstd::set_ck_stream_arm(true);
        // force_ignore_checksum must still work on both arms
        let _ = rusty_zstd::decompress_with(&z, DecompressOptions{force_ignore_checksum:true, ..Default::default()}).unwrap();
        okg += good_ok as i32; okc += corrupt_caught as i32; n += 1;
        tf += t[0]; ts += t[1];
        println!("{:<13} {:>10.3} {:>10.3} {:>8.2}% | {:>8} {:>8}", id, t[0], t[1], (t[1]/t[0]-1.0)*100.0,
            if good_ok {"ok"} else {"FAIL"}, if corrupt_caught {"caught"} else {"MISSED"});
    }
    println!("\n  TOTAL decode {:.3} -> {:.3} ms ({:+.2}%)", tf, ts, (ts/tf-1.0)*100.0);
    println!("  good frames decoded correctly on both arms: {okg}/{n}");
    println!("  corrupt frames rejected on both arms:       {okc}/{n}");
    assert_eq!(okg, n); assert_eq!(okc, n);
}
