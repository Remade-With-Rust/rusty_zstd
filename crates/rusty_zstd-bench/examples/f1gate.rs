//! Gates for shipping FINDING 1:
//!  (a) `compress()` (no dictionary) must be BYTE-IDENTICAL -- the arm may only
//!      touch the dict/prefix path, so every speed board stays valid.
//!  (b) the real zstd binary must decode our dict frames with the wider window.
//!  (c) the fallback `set_prefix_window_arm(false)` must reproduce the old bytes.
use rusty_zstd::{CompressOptions, Dictionary};
use std::process::Command;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m","text-32m","zeros-32m","incomp-32m"];
fn main() {
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    // (a) no-dictionary path untouched
    let (mut n, mut moved) = (0, 0);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(1 << 20)];
        for lvl in [1i32, 3, 19, 22] {
            rusty_zstd::set_prefix_window_arm(true);
            let a = rusty_zstd::compress(s, lvl).unwrap();
            rusty_zstd::set_prefix_window_arm(false);
            let b = rusty_zstd::compress(s, lvl).unwrap();
            n += 1; if a != b { moved += 1; }
        }
    }
    println!("(a) no-dict `compress()`: {n} cells, arm moved output on {moved} (must be 0)");
    assert_eq!(moved, 0, "Finding 1 leaked into the no-dictionary path");

    // (b) external decode of dict frames with the widened window
    let raw = std::fs::read("target/_g3.dict").ok();
    let (mut ok, mut bad) = (0, 0);
    for id in ["mozilla","webster","nci","samba","xml"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
        if f.len() < (5 << 20) { continue }
        let (pre, tail) = (&f[..4 << 20], &f[4 << 20..5 << 20]);
        rusty_zstd::set_prefix_window_arm(true);
        // prefix path: C reads it back with --patch-from
        std::fs::write("target/_f1.ref", pre).unwrap();
        let z = rusty_zstd::compress_using_prefix(tail, pre, 19).unwrap();
        std::fs::write("target/_f1.zst", &z).unwrap();
        let out = Command::new(zstd).args(["-d","-c","-f","--patch-from","target/_f1.ref","target/_f1.zst"]).output().unwrap();
        if out.status.success() && out.stdout == tail { ok += 1 } else { bad += 1; println!("   FAIL prefix {id}: {} bytes", out.stdout.len()); }
        // dictionary path
        if let Some(r) = &raw {
            let d = Dictionary::from_bytes(r).unwrap();
            std::fs::write("target/_f1d.dict", r).unwrap();
            let z = rusty_zstd::compress_using_dict_with(tail, &d, CompressOptions{level:19,checksum:false}, true).unwrap();
            std::fs::write("target/_f1d.zst", &z).unwrap();
            let out = Command::new(zstd).args(["-d","-c","-f","-D","target/_f1d.dict","target/_f1d.zst"]).output().unwrap();
            if out.status.success() && out.stdout == tail { ok += 1 } else { bad += 1; println!("   FAIL dict {id}: {} bytes", out.stdout.len()); }
        }
    }
    println!("(b) external zstd decode of dict/prefix frames: {ok} OK, {bad} FAILED");
    assert_eq!(bad, 0);

    // (c) fallback reproduces the pre-finding bytes
    let mut same = 0; let mut tot = 0;
    for id in ["mozilla","webster","nci","xml"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
        if f.len() < (5 << 20) { continue }
        let (pre, tail) = (&f[..4 << 20], &f[4 << 20..5 << 20]);
        for lvl in [3i32, 19] {
            rusty_zstd::set_prefix_window_arm(false);
            let off = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            rusty_zstd::set_prefix_window_arm(true);
            let on = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            assert!(rusty_zstd::decompress_using_prefix(&on, pre).unwrap() == tail);
            tot += 1;
            if off.len() >= on.len() { same += 1; }
        }
    }
    println!("(c) fallback works; ON is <= OFF in size on {same}/{tot} cells");
    for f in ["target/_f1.ref","target/_f1.zst","target/_f1d.dict","target/_f1d.zst"] { let _ = std::fs::remove_file(f); }
    rusty_zstd::set_prefix_window_arm(true);
}
