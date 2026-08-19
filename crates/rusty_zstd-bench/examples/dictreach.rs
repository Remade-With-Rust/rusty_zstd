//! Is Finding 1 a performance TRADE or a CORRECTNESS gap?
//!
//! If a caller hands us a 4 MiB dictionary and the window is sized to the 1 MiB
//! payload, matches are rejected at `ip - m > window` -- so most of the supplied
//! dictionary is UNREACHABLE. The question that decides how to judge it: does
//! the encoder actually reference the older part of the dictionary when the
//! window allows it?
//!
//! Probe: compress the SAME payload against dictionaries built from the SAME
//! bytes, truncated to different lengths. If output stops changing beyond
//! `window`, the tail of the dictionary is provably ignored.
use rusty_zstd::Dictionary;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    const PRE: usize = 4 << 20; const PAY: usize = 1 << 20;
    println!("DICTIONARY REACHABILITY @ L{lvl} — payload {} KiB, dictionary truncated from the FRONT", PAY>>10);
    println!("{:<13} {:>8} | {:>10} {:>10} {:>10} {:>10} {:>10}", "corpus", "wlog", "dict 4M", "dict 2M", "dict 1M", "dict 512K", "dict 256K");
    let mut ignored = 0; let mut n = 0;
    for id in ["mozilla","webster","nci","samba","osdb","dickens","xml","reymont"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
        if f.len() < PRE + PAY { continue }
        let tail = &f[PRE..PRE+PAY];
        let p = rusty_zstd::compression_params(lvl, Some(PAY as u64)).unwrap();
        let mut row = format!("{:<13} {:>8} |", id, p.window_log);
        let mut sizes = Vec::new();
        for dk in [PRE, PRE/2, PRE/4, PRE/8, PRE/16] {
            // keep the bytes NEAREST the payload; truncate the far end
            let d = Dictionary::raw(f[PRE-dk..PRE].to_vec());
            let z = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
            assert!(rusty_zstd::decompress_using_dict(&z, &d).unwrap() == tail);
            sizes.push(z.len());
            row += &format!(" {:>10}", z.len());
        }
        n += 1;
        // if 4M, 2M and 1M all agree, everything beyond the window was ignored
        if sizes[0] == sizes[1] && sizes[1] == sizes[2] { ignored += 1; }
        println!("{row}");
    }
    println!("\n  corpora where a 4 MiB, 2 MiB and 1 MiB dictionary give IDENTICAL output: {ignored}/{n}");
    println!("  identical => the bytes beyond `window` were never referenced, i.e. the");
    println!("  caller-supplied dictionary is silently truncated to the payload-sized window.");
}
