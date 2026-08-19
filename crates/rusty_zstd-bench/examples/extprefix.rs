//! External gate for the prefix path: a frame we produce with `--patch-from`
//! semantics must decode in the REAL zstd binary, not just in our decoder.
fn main() {
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let (mut ok, mut bad) = (0, 0);
    for id in ["mozilla","webster","samba","nci","versions-16m","osdb","xml"] {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < (5 << 20) { continue }
        let pre = &full[..4 << 20];
        let tail = &full[4 << 20..(4 << 20) + (1 << 20)];
        let rf = format!("target/_pref_{id}.ref");
        std::fs::write(&rf, pre).unwrap();
        for lvl in [1i32, 3, 19] {
            let z = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            let zf = format!("target/_pref_{id}_{lvl}.zst");
            std::fs::write(&zf, &z).unwrap();
            let out = std::process::Command::new(zstd)
                .args(["-d", "-c", "-f", "--patch-from", &rf, &zf]).output().unwrap();
            if out.status.success() && out.stdout == tail { ok += 1 } else {
                bad += 1;
                println!("  FAIL {id} L{lvl}: status {:?}, {} vs {} bytes", out.status.code(), out.stdout.len(), tail.len());
            }
            let _ = std::fs::remove_file(&zf);
        }
        let _ = std::fs::remove_file(&rf);
    }
    println!("external zstd -d --patch-from: {ok} OK, {bad} FAILED");
    assert_eq!(bad, 0, "external decode of the prefix path FAILED");
}
