//! GATE 3 @ L22 — write_dict_id. Steps 1 and 2 only; no clock, because a 4-byte
//! header field cannot have a speed component and the L1/L3 runs already showed
//! the timing column is pure scatter.
//!
//! Step 1: the default must differ from the value set (needs a NON-ZERO dict id,
//!         or both arms emit identical frames and the A/B is null).
//! Step 2: does the outcome differ by CONTENT? If the delta is the same on every
//!         corpus there is nothing to dispatch on.
use rusty_zstd::{CompressOptions, Dictionary};
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn with_id(raw: &[u8], id: u32) -> Dictionary {
    let mut v = raw.to_vec();
    v[4..8].copy_from_slice(&id.to_le_bytes());
    Dictionary::from_bytes(&v).expect("dict")
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(22);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(256 << 10);
    let raw = std::fs::read("target/_g3.dict").expect("run dtrain first");
    let d = with_id(&raw, 0x00C0FFEE);
    println!("GATE 3 @ L{lvl} — dict id {:#x}, cap {} KiB", d.id(), cap >> 10);
    println!("{:<13} {:>11} {:>11} {:>7}", "corpus", "write-id", "no-id", "delta");
    let mut deltas = std::collections::BTreeSet::new();
    let mut n = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let go = |w: bool| rusty_zstd::compress_using_dict_with(s, &d, CompressOptions{level:lvl,checksum:false}, w).unwrap();
        let (a, b) = (go(true), go(false));
        assert!(rusty_zstd::decompress_using_dict(&a, &d).unwrap() == s, "{id}: round-trip");
        let delta = a.len() as i64 - b.len() as i64;
        deltas.insert(delta);
        n += 1;
        println!("{:<13} {:>11} {:>11} {:>7}", id, a.len(), b.len(), delta);
    }
    println!("\n  STEP 1: distinct deltas across {n} corpora: {:?}", deltas);
    println!("  STEP 2: {}", if deltas.len() == 1 {
        "the outcome is IDENTICAL on every content type -> nothing to dispatch on -> CONSTANT"
    } else { "outcome varies by content -> candidate for dispatch" });
}
