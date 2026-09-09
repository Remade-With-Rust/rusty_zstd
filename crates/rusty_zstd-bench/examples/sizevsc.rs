//! SIZE-ONLY head-to-head vs C zstd. No clock, so a loaded box cannot corrupt
//! it -- compressed bytes are the same number under any load.
//!
//! Shows our output before and after the row-finder AUTO gate against the same
//! C reference, so the win is priced against zstd rather than against ourselves.
use std::process::Command;
use rusty_zstd as rz;
const SIL: &[&str] = &["mr","ooffice","osdb","reymont","sao","webster","dickens",
                       "mozilla","nci","samba","xml","x-ray"];
fn c_size(zstd: &str, path: &str, lvl: i32) -> Option<usize> {
    let o = Command::new(zstd).args(["-q", "-f", &format!("-{lvl}"), "--no-check",
                                     path, "-o", &format!("{path}.zst")]).output().ok()?;
    let _ = o;
    let n = std::fs::metadata(format!("{path}.zst")).ok()?.len() as usize;
    let _ = std::fs::remove_file(format!("{path}.zst"));
    Some(n)
}
fn main() {
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    let tmp = std::env::temp_dir().join("sizevsc.bin");
    println!("SILESIA, cap {} KiB, vs zstd 1.5.7 -- SIZE ONLY (load-immune)\n", cap >> 10);
    println!("{:<10}{:>4}{:>11}{:>11}{:>11}{:>10}{:>10}", "corpus", "L", "C", "us(old)", "us(new)", "old/C", "new/C");
    println!("{}", "-".repeat(68));
    for lvl in [7i32, 9] {
        let (mut sc, mut so, mut sn) = (0u64, 0u64, 0u64);
        for id in SIL {
            let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
            let src = &f[..f.len().min(cap)];
            std::fs::write(&tmp, src).unwrap();
            let Some(c) = c_size(zstd, tmp.to_str().unwrap(), lvl) else { continue };
            let p = rz::compression_params(lvl, Some(src.len() as u64)).unwrap();
            rz::set_row_arm(false);
            let old = rz::compress_with_params(src, p, false).unwrap().len();
            rz::set_row_arm_auto();
            let new = rz::compress_with_params(src, p, false).unwrap().len();
            sc += c as u64; so += old as u64; sn += new as u64;
            println!("{:<10}{:>4}{:>11}{:>11}{:>11}{:>10.4}{:>10.4}",
                     id, lvl, c, old, new, old as f64 / c as f64, new as f64 / c as f64);
        }
        println!("{:<10}{:>4}{:>11}{:>11}{:>11}{:>10.4}{:>10.4}   <== L{lvl} TOTAL",
                 "TOTAL", lvl, sc, so, sn, so as f64 / sc as f64, sn as f64 / sc as f64);
        println!();
    }
    let _ = std::fs::remove_file(&tmp);
}
