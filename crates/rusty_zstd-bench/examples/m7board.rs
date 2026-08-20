//! Regenerate sections 1 and 2 of m7-anatomy.md at that document's OWN protocol.
//!
//! `runallfour` exists but runs 9 iterations against a 350 ms budget; m7-anatomy
//! claims N >= 25 per phase. Using the lighter harness and presenting it under
//! the heavier claim would be a work-parity break in the instrument, so this
//! runs the stated protocol:
//!
//!   * best-of-N both arms, N >= 25 per phase, warmup discarded
//!   * phases timed separately, as C's `-b` does
//!   * decompress into a REUSED buffer via `decompress_into`
//!   * CHECKSUM PARITY: our compress runs with checksum OFF, matching
//!     `ZSTD_c_checksumFlag = 0` in `zstd -b`. This changes the MEASUREMENT,
//!     not the product -- the shipped default is still `checksum: true`.
//!   * a per-row SAME-ARM spread, so every number is quoted beside its own noise
use std::process::Command;
use std::time::Instant;

const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const N: usize = 25;

fn c_arm(zstd: &str, path: &str, lvl: i32) -> Option<(f64, f64, usize)> {
    let out = Command::new(zstd)
        .args(["--ultra", &format!("-b{lvl}"), "-i1", "-T1", path])
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout);
    let line = raw.split(|c: char| c.is_control())
        .filter(|l| l.matches("MB/s").count() >= 2 && l.contains("->"))
        .next_back()?.to_string();
    let csize: usize = line.split("->").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let mut sp = Vec::new();
    for seg in line.split("MB/s") {
        if let Some(t) = seg.split_whitespace().next_back() {
            if let Ok(v) = t.trim_end_matches(',').parse::<f64>() { sp.push(v); }
        }
    }
    if sp.len() < 2 { return None }
    Some((sp[0], sp[1], csize))
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let tmp = std::env::temp_dir().join("m7board.bin");
    let mut rows: Vec<(String, f64, f64, f64, f64, f64, f64)> = Vec::new();
    let mut null_worst = 0.0f64;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        std::fs::write(&tmp, s).unwrap();
        let Some((cc, cd, csize)) = c_arm(zstd, tmp.to_str().unwrap(), lvl) else {
            eprintln!("skip {id}: C arm failed"); continue };
        let p = rusty_zstd::compression_params(lvl, Some(s.len() as u64)).unwrap();
        // CHECKSUM PARITY with `zstd -b`
        let z = rusty_zstd::compress_with_params(s, p, false).unwrap();
        let mb = s.len() as f64 / 1_048_576.0;
        // encode, best-of-N, warmup discarded; a second best-of-N is the null arm
        let mut enc = [f64::MAX; 2];
        for arm in 0..2 {
            let _ = rusty_zstd::compress_with_params(s, p, false).unwrap();
            for _ in 0..N {
                let t = Instant::now();
                let q = rusty_zstd::compress_with_params(s, p, false).unwrap();
                let e = t.elapsed().as_secs_f64();
                std::hint::black_box(q.len());
                if e < enc[arm] { enc[arm] = e }
            }
        }
        // decode into a REUSED buffer, as C's -b does
        let mut buf = Vec::with_capacity(s.len());
        let mut dec = f64::MAX;
        let _ = rusty_zstd::decompress_into(&mut buf, &z).unwrap();
        for _ in 0..N {
            buf.clear();
            let t = Instant::now();
            let _ = rusty_zstd::decompress_into(&mut buf, &z).unwrap();
            let e = t.elapsed().as_secs_f64();
            if e < dec { dec = e }
        }
        assert!(buf == s, "{id}: round-trip");
        let uc = mb / enc[0];
        let ud = mb / dec;
        let spread = (enc[0].max(enc[1]) / enc[0].min(enc[1]) - 1.0) * 100.0;
        if spread > null_worst { null_worst = spread }
        rows.push(((*id).to_string(), cc, uc, cd, ud, z.len() as f64 / csize as f64, spread));
    }
    rows.sort_by(|a, b| a.5.partial_cmp(&b.5).unwrap());
    println!("| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |");
    println!("| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |");
    let (mut sc, mut sd, mut sr) = (0.0, 0.0, 0.0);
    let (mut wc, mut wd, mut wr) = (0, 0, 0);
    let (mut worst_r, mut worst_id) = (0.0f64, String::new());
    for (id, cc, uc, cd, ud, r, _sp) in &rows {
        let rc = cc / uc; let rd = cd / ud;
        if rc < 1.0 { wc += 1 } if rd < 1.0 { wd += 1 } if *r < 1.0 { wr += 1 }
        if *r > worst_r { worst_r = *r; worst_id = id.clone() }
        sc += rc; sd += rd; sr += r;
        let b = |v: f64, c: bool| if c { format!("**{v:.2}**") } else { format!("{v:.2}") };
        println!("| {:<12} | {:>7.1} | {:>7.1} | {:>10} | {:>8.1} | {:>9.1} | {:>10} | {:>9} |",
            id, cc, uc, b(rc, rc < 1.0), cd, ud, b(rd, rd < 1.0),
            if *r < 1.0 { format!("**{r:.3}**") } else { format!("{r:.3}") });
    }
    let n = rows.len() as f64;
    println!("\n**mean C/us comp {:.2}, decomp {:.2} | mean ratio {:.3} | worst ratio {:.3} ({worst_id}) | we beat C: {wc} comp, {wd} decomp, {wr} ratio**",
        sc / n, sd / n, sr / n, worst_r);
    println!("\nSESSION NULL ARM (worst same-arm encode spread): {null_worst:.2}%");
}
