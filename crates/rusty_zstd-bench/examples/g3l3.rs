//! GATE 3 @ L3 — `write_dict_id`: CONSTANT or DISPATCH?
//!
//! The gate selects whether the Dictionary_ID field appears in the frame header.
//! DEAD CHECK needs a dictionary whose id is NON-ZERO, because
//! `dict.map(Dictionary::id).filter(|&id| id != 0)` makes a raw dict (id 0)
//! produce identical frames on both arms -- a null A/B.
//!
//! The id is at bytes 4..8 of a trained dictionary, so the three RFC 8878
//! DID_Field_Size buckets (1 / 2 / 4 bytes) can be exercised by patching it
//! rather than by training three dictionaries.
use rusty_zstd::{CompressOptions, Dictionary};
use std::process::Command;
use std::time::Instant;

const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m","text-32m","zeros-32m","incomp-32m"];

const CAP: usize = 1 << 20;

fn with_id(raw: &[u8], id: u32) -> Dictionary {
    let mut v = raw.to_vec();
    v[4..8].copy_from_slice(&id.to_le_bytes());
    Dictionary::from_bytes(&v).expect("patched dict")
}

fn main() {
    #[allow(non_snake_case)]
    let LVL: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let raw = std::fs::read("target/_g3.dict").expect("run `dtrain` first");
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";

    // ---- 1. DID field size per RFC bucket ----
    println!("1. DID_Field_Size buckets (RFC 8878): flag 1/2/3 -> 1/2/4 bytes");
    let probe = vec![b'a'; 4096];
    for (id, want) in [(1u32, 1usize), (255, 1), (256, 2), (65535, 2), (65536, 4), (0x00C0FFEE, 4)] {
        let d = with_id(&raw, id);
        let on = rusty_zstd::compress_using_dict_with(&probe, &d, CompressOptions { level: LVL, checksum: false }, true).unwrap();
        let off = rusty_zstd::compress_using_dict_with(&probe, &d, CompressOptions { level: LVL, checksum: false }, false).unwrap();
        let got = on.len() - off.len();
        let hdr = match rusty_zstd::get_frame_header(&on).unwrap() {
            rusty_zstd::FrameKind::Zstd(h) => h,
            _ => panic!("not a zstd frame"),
        };
        println!("   id {:>10} -> +{got} byte(s) [want {want}] {}  header dict_id {:?}",
            id, if got == want { "OK" } else { "MISMATCH" }, hdr.dict_id);
        assert_eq!(got, want, "DID field size wrong for id {id}");
        assert_eq!(hdr.dict_id, Some(id));
        assert!(rusty_zstd::decompress_using_dict(&on, &d).unwrap() == probe);
        assert!(rusty_zstd::decompress_using_dict(&off, &d).unwrap() == probe);
    }

    // ---- 2. C accepts both arms ----
    println!("\n2. external `zstd -d -D dict` on both arms");
    std::fs::write("target/_g3c.dict", &raw).unwrap();
    let d = with_id(&raw, 0x00C0FFEE);
    let src = std::fs::read("corpora/data/silesia/xml").unwrap();
    let src = &src[..src.len().min(CAP)];
    for (label, w) in [("write-id", true), ("no-id", false)] {
        let z = rusty_zstd::compress_using_dict_with(src, &d, CompressOptions { level: LVL, checksum: false }, w).unwrap();
        std::fs::write("target/_g3.zst", &z).unwrap();
        let out = Command::new(zstd).args(["-d", "-c", "-f", "-D", "target/_g3c.dict", "target/_g3.zst"]).output().unwrap();
        println!("   {label:<9} {} bytes -> C decode {}", z.len(),
            if out.status.success() && out.stdout == src { "OK".to_string() } else { format!("FAILED ({} bytes)", out.stdout.len()) });
    }

    // ---- 3. the gate itself, all 18 corpora ----
    println!("\n3. CONSTANT TEST — 18 corpora, L{LVL}");
    println!("{:<13} {:>11} {:>11} {:>7} | {:>9} {:>9} {:>8}", "corpus", "write-id B", "no-id B", "delta", "on ms", "off ms", "time%");
    let (mut tb, mut nb, mut ton, mut toff) = (0i64, 0i64, 0.0f64, 0.0f64);
    let mut bad = 0;
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &full[..full.len().min(CAP)];
        let go = |w: bool| rusty_zstd::compress_using_dict_with(s, &d, CompressOptions { level: LVL, checksum: false }, w).unwrap();
        let (a, b) = (go(true), go(false));
        let bench = |w: bool| { let mut best = f64::MAX; for _ in 0..9 { let t = Instant::now(); let _ = go(w); let e = t.elapsed().as_secs_f64()*1000.0; if e < best { best = e; } } best };
        let (ta, tbb) = (bench(true), bench(false));
        assert!(rusty_zstd::decompress_using_dict(&a, &d).unwrap() == s, "{id}: round-trip");
        if a.len() - b.len() != 4 { bad += 1; }
        tb += a.len() as i64; nb += b.len() as i64; ton += ta; toff += tbb;
        println!("{:<13} {:>11} {:>11} {:>7} | {:>9.3} {:>9.3} {:>7.2}%", id, a.len(), b.len(), a.len() as i64 - b.len() as i64, ta, tbb, (tbb/ta-1.0)*100.0);
    }
    println!("\n  size {tb} -> {nb} ({:+.5}%) | cells whose delta != 4 bytes: {bad}", (nb as f64/tb as f64-1.0)*100.0);
    println!("  time {ton:.1} -> {toff:.1} ms ({:+.2}%)", (toff/ton-1.0)*100.0);
    let _ = std::fs::remove_file("target/_g3.zst");
    let _ = std::fs::remove_file("target/_g3c.dict");
}
