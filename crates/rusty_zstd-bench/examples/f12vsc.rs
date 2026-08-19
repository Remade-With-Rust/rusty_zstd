//! FINDINGS 1+2 judged against C, not against our own no-tree baseline.
//!
//! The "+241% time" verdict compared the findings to a codec that does NOT build
//! a tree over the prefix. C DOES (verified: 15.4x scaling at L19 with a growing
//! reference). The campaign's shipping pair is `us/c size` and `C/us compress`,
//! so the question that decides shipping is where we land against C -- with the
//! findings OFF and ON.
use rusty_zstd::Dictionary;
use std::process::Command;
use std::time::Instant;

const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20;
const PAY: usize = 1 << 20;

fn c_patch(zstd: &str, rf: &str, pf: &str, of: &str, lvl: i32) -> Option<(f64, usize)> {
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let st = Command::new(zstd).args(["--ultra", &format!("-{lvl}"), "-f", "-q", "--patch-from", rf, pf, "-o", of]).output().ok()?;
        if !st.status.success() { return None; }
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < best { best = e; }
    }
    Some((best, std::fs::metadata(of).ok()?.len() as usize))
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    println!("FINDINGS 1+2 vs C --patch-from @ L{lvl} (ref {} MiB, payload {} MiB)", PRE>>20, PAY>>20);
    println!("{:<13} {:>9} {:>9} {:>9} | {:>8} {:>8} | {:>8} {:>8} | {:>8} {:>8}",
        "corpus", "C ms", "off ms", "on ms", "us/c off", "us/c on", "C/us off", "C/us on", "sz off", "sz on");
    let (mut co, mut uo, mut un) = (0.0f64, 0.0f64, 0.0f64);
    let (mut sc, mut so, mut sn) = (0i64, 0i64, 0i64);
    let (mut winc_off, mut winc_on) = (0, 0);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PRE + PAY { continue }
        let pre = &full[..PRE];
        let tail = &full[PRE..PRE+PAY];
        let (rf, pf, of) = (format!("target/_f_{id}.ref"), format!("target/_f_{id}.pay"), format!("target/_f_{id}.zst"));
        std::fs::write(&rf, pre).unwrap(); std::fs::write(&pf, tail).unwrap();
        let Some((cms, csz)) = c_patch(zstd, &rf, &pf, &of, lvl) else { println!("{id}: C failed"); continue };
        let d = Dictionary::raw(pre.to_vec());
        let bench = |on: bool| {
            rusty_zstd::set_prime_bt_tree_arm(on);
            rusty_zstd::set_prefix_window_arm(on);
            if on { rusty_zstd::set_prime_bt_depth_arm(5); }
            let z = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
            let mut best = f64::MAX;
            for _ in 0..3 { let t = Instant::now(); let _ = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap(); let e = t.elapsed().as_secs_f64()*1000.0; if e < best { best = e; } }
            assert!(rusty_zstd::decompress_using_dict(&z, &d).unwrap() == tail, "{id}: round-trip");
            (best, z.len())
        };
        let (toff, zoff) = bench(false);
        let (ton, zon) = bench(true);
        rusty_zstd::set_prime_bt_tree_arm(false); rusty_zstd::set_prefix_window_arm(false);
        let _ = std::fs::remove_file(&rf); let _ = std::fs::remove_file(&pf); let _ = std::fs::remove_file(&of);
        if cms/toff > 1.0 { winc_off += 1 } if cms/ton > 1.0 { winc_on += 1 }
        co += cms; uo += toff; un += ton; sc += csz as i64; so += zoff as i64; sn += zon as i64;
        println!("{:<13} {:>9.0} {:>9.0} {:>9.0} | {:>8.3} {:>8.3} | {:>8.2} {:>8.2} | {:>8} {:>8}",
            id, cms, toff, ton, zoff as f64/csz as f64, zon as f64/csz as f64, cms/toff, cms/ton, zoff, zon);
    }
    println!("\n  us/c size   OFF {:.4}   ON {:.4}", so as f64/sc as f64, sn as f64/sc as f64);
    println!("  C/us time   OFF {:.2}    ON {:.2}   (>1 = we are faster than C)", co/uo, co/un);
    println!("  we beat C on time: OFF {winc_off}, ON {winc_on} of 15");
}
