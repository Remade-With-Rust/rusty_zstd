//! GATE 2 @ L19 — is the prefix actually USABLE by the Bt ladder?
//!
//! `prime_tables` writes hash HEADS only. From BtLazy2 up the finder descends a
//! binary tree held in `chain`, and priming never builds a node -- so the first
//! descent from a primed head reads an unseeded child and terminates at once.
//! libzstd does the opposite: `ZSTD_loadDictionaryContent` calls `ZSTD_updateTree`
//! for btlazy2/btopt/btultra/btultra2, under the comment "we want the dictionary
//! table fully sorted".
//!
//! If that gap costs us, our `--patch-from` ratio at L19 should trail C's by more
//! than it does at L1/L3, where no tree exists to build.
use std::process::Command;
fn main() {
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
    const PRE: usize = 4 << 20;
    const PAY: usize = 1 << 20;
    for &lvl in &[1i32, 3, 19] {
        println!("\n=== L{lvl}: our --patch-from vs C's ===");
        println!("{:<13} {:>11} {:>11} {:>9}   {:>11} {:>9}", "corpus", "us bytes", "C bytes", "us/c", "no-pref us", "pref gain");
        let (mut wins, mut n, mut tot_u, mut tot_c) = (0, 0, 0i64, 0i64);
        for id in IDS {
            let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            if full.len() < PRE + PAY { continue }
            let pre = &full[..PRE];
            let tail = &full[PRE..PRE + PAY];
            let rf = format!("target/_c19_{id}.ref");
            let pf = format!("target/_c19_{id}.pay");
            let of = format!("target/_c19_{id}.zst");
            std::fs::write(&rf, pre).unwrap();
            std::fs::write(&pf, tail).unwrap();
            let st = Command::new(zstd)
                .args(["--ultra", &format!("-{lvl}"), "-f", "--patch-from", &rf, &pf, "-o", &of])
                .output().unwrap();
            let csz = std::fs::metadata(&of).map(|m| m.len() as usize).unwrap_or(0);
            if !st.status.success() || csz == 0 { println!("{id}: C failed"); continue }
            let us = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len();
            let nopref = rusty_zstd::compress(tail, lvl).unwrap().len();
            let _ = std::fs::remove_file(&rf); let _ = std::fs::remove_file(&pf); let _ = std::fs::remove_file(&of);
            let ratio = us as f64 / csz as f64;
            if ratio < 1.0 { wins += 1 }
            tot_u += us as i64; tot_c += csz as i64;
            n += 1;
            println!("{:<13} {:>11} {:>11} {:>9.3}   {:>11} {:>8.1}%", id, us, csz, ratio, nopref,
                (us as f64 / nopref as f64 - 1.0) * 100.0);
        }
        println!("  total us {tot_u} / C {tot_c} = {:.4} | we are smaller on {wins}/{n}",
            tot_u as f64 / tot_c as f64);
    }
}
