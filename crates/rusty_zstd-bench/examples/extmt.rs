//! External gate for the MT path: our multi-frame output must decode in the
//! REAL zstd binary, not just in our decoder.
use rusty_zstd::AdvancedOptions;
fn main() {
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let mut ok = 0; let mut bad = 0;
    for id in ["mozilla","webster","samba","nci","versions-16m","zeros-32m","incomp-32m"] {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(24 << 20)];
        let params = rusty_zstd::compression_params(3, Some(src.len() as u64)).unwrap();
        for (w, job) in [(4u32, 512usize << 10), (8, 0)] {
            let adv = AdvancedOptions { nb_workers: w, job_size: job, ..Default::default() };
            let z = rusty_zstd::compress_with_advanced(src, params, false, None, &[], true, adv).unwrap();
            let f = format!("target/_extmt_{id}_{w}.zst");
            std::fs::write(&f, &z).unwrap();
            let out = std::process::Command::new(zstd).args(["-d", "-c", "-f", &f]).output().unwrap();
            let good = out.status.success() && out.stdout == src;
            if good { ok += 1 } else { bad += 1; println!("  FAIL {id} w={w} job={job}: status {:?}, {} vs {} bytes", out.status.code(), out.stdout.len(), src.len()); }
            let _ = std::fs::remove_file(&f);
        }
    }
    println!("external zstd -d on MT frames: {ok} OK, {bad} FAILED");
    assert_eq!(bad, 0, "external decode of the MT path FAILED");
}
