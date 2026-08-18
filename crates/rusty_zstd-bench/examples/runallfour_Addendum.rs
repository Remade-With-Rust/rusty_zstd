//! RUN ALL FOUR (ADDENDUM) — one command for the whole gate-outcome cycle of
//! the frame/block-level campaign in `docs/plans/gg-Addendum.md`.
//!
//!   cargo run --release -p rusty_zstd-bench --example runallfour_Addendum
//!   cargo run --release -p rusty_zstd-bench --example runallfour_Addendum -- 19 "CONSTANT 128 KiB"
//!
//! 1. Runs the full encode+decode board at L1, L3, L19 and L22 over all 18
//!    corpora, round-trip asserted per corpus.
//! 2. Rewrites the four charts under `## OUTCOMES FROM GATES (RUN ALL FOUR)` in
//!    `docs/plans/gg-Addendum.md`, in place.
//! 3. Appends the next gate's verdict beneath the previous one.
//!
//! Args: `[gate_number] [verdict text]`. With no args it refreshes the boards
//! and leaves the verdict lines untouched.
//!
//! # GATE 4 — CHECKSUM PARITY, and why this board differs from `runallfour`
//!
//! `rusty_zstd::compress(src, lvl)` sets `checksum: true` (it matches the zstd
//! CLI, which is what a crate user expects). `zstd -b` — the C arm every board
//! here uses — runs WITHOUT `--check`, matching libzstd's
//! `ZSTD_c_checksumFlag = 0` default. Timing the two against each other charges
//! us a full xxh64 pass over every byte that C never runs.
//!
//! `runallfour.rs` and `gateboard.rs` call `compress(src, lvl)`, so every board
//! in `gg-matchfind.md` carries that tax. This board does NOT: the `us comp`
//! column is measured through `compress_with(.., checksum: false)`, which is
//! work-count parity with the C arm. The checksum tax is measured separately
//! and reported on the summary line, so nothing is hidden — the two campaigns'
//! boards stay reconcilable by a single stated number per level.
//!
//! Gate 4 of `gg-Addendum.md` is exactly this, and it is why that row is an
//! assertion rather than a dispatch.
use rusty_zstd::CompressOptions;
use std::io::Write;
use std::process::Command;
use std::time::Instant;

const MAX_ITERS: usize = 9;
const BUDGET_MS: f64 = 350.0;
const DOC: &str = "docs/plans/gg-Addendum.md";
/// (level, prefix bytes) — 2 MiB at the high levels keeps each board ~65 s.
/// Identical to `runallfour.rs`, so the two campaigns' boards line up.
const LEVELS: &[(i32, usize)] = &[
    (1, 8 * 1024 * 1024),
    (3, 8 * 1024 * 1024),
    (19, 2 * 1024 * 1024),
    (22, 2 * 1024 * 1024),
];
const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];

/// Work-count parity with `zstd -b`: no content checksum. See the module note.
fn enc(src: &[u8], lvl: i32) -> Vec<u8> {
    rusty_zstd::compress_with(
        src,
        CompressOptions {
            level: lvl,
            checksum: false,
        },
    )
    .unwrap()
}

/// The arm `runallfour.rs` measures — kept only to price the tax.
fn enc_ck(src: &[u8], lvl: i32) -> Vec<u8> {
    rusty_zstd::compress(src, lvl).unwrap()
}

fn best_of<F: FnMut() -> usize>(mut f: F) -> (f64, usize) {
    let t0 = Instant::now();
    let (mut best, mut out) = (f64::MAX, 0usize);
    for _ in 0..MAX_ITERS {
        let t = Instant::now();
        out = f();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < best {
            best = e;
        }
        if t0.elapsed().as_secs_f64() * 1000.0 > BUDGET_MS {
            break;
        }
    }
    (best, out)
}

fn c_arm(zstd: &str, path: &str, lvl: i32) -> Option<(f64, f64, usize)> {
    let out = Command::new(zstd)
        .args(["--ultra", &format!("-b{lvl}"), "-i1", "-T1", path])
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout);
    // `zstd -b` rewrites ONE line with carriage returns; keep the last COMPLETE
    // record, the only one carrying both MB/s figures.
    let line = raw
        .split(|c: char| c.is_control())
        .filter(|l| l.matches("MB/s").count() >= 2 && l.contains("->"))
        .next_back()?
        .to_string();
    let csize: usize = line.split("->").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let mut sp = Vec::new();
    for seg in line.split("MB/s") {
        if let Some(t) = seg.split_whitespace().next_back() {
            if let Ok(v) = t.trim_end_matches(',').parse::<f64>() {
                sp.push(v);
            }
        }
    }
    if sp.len() < 2 {
        return None;
    }
    Some((sp[0], sp[1], csize))
}

struct Row {
    id: String,
    cc: f64,
    uc: f64,
    cd: f64,
    ud: f64,
    ratio: f64,
    /// checksum-on encode ms / checksum-off encode ms − 1, in percent.
    ck_tax: f64,
}

fn board(lvl: i32, cap: usize, zstd: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for id in IDS {
        let full = match std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        {
            Ok(v) => v,
            Err(_) => continue,
        };
        let src = &full[..full.len().min(cap)];
        let tmp = format!("target/_a4_{id}.bin");
        if std::fs::write(&tmp, src).is_err() {
            continue;
        }
        let mb = src.len() as f64 / 1_048_576.0;
        // PARITY arm — no checksum, matching `zstd -b`.
        let (cms, csz) = best_of(|| enc(src, lvl).len());
        // GATE 4 instrument: what the checksum costs on this corpus at this level.
        let (cms_ck, _) = best_of(|| enc_ck(src, lvl).len());
        let z = enc(src, lvl);
        let mut dst = Vec::with_capacity(src.len());
        let (dms, _) = best_of(|| {
            dst.clear();
            rusty_zstd::decompress_into(&mut dst, &z).unwrap();
            dst.len()
        });
        assert_eq!(dst, src, "{id}: ROUND-TRIP FAILED at L{lvl}");
        let Some((cc, cd, ccsz)) = c_arm(zstd, &tmp, lvl) else {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("  (C arm failed for {id} @ L{lvl})");
            continue;
        };
        let _ = std::fs::remove_file(&tmp);
        rows.push(Row {
            id: (*id).into(),
            cc,
            uc: mb / (cms / 1000.0),
            cd,
            ud: mb / (dms / 1000.0),
            ratio: csz as f64 / ccsz as f64,
            ck_tax: (cms_ck / cms - 1.0) * 100.0,
        });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    rows.sort_by(|a, b| a.ratio.partial_cmp(&b.ratio).unwrap());
    rows
}

fn bold(v: f64) -> String {
    if v < 1.0 {
        format!("**{v:.2}**")
    } else {
        format!("{v:.2}")
    }
}
fn boldr(v: f64) -> String {
    if v < 1.0 {
        format!("**{v:.3}**")
    } else {
        format!("{v:.3}")
    }
}

fn render(rows: &[Row], secs: f64) -> String {
    let mut s = String::new();
    s.push_str("| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |\n");
    s.push_str("| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |\n");
    for r in rows {
        s.push_str(&format!(
            "| {:<12} | {:>7.1} | {:>7.1} | {:>10} | {:>8.1} | {:>9.1} | {:>10} | {:>9} |\n",
            r.id,
            r.cc,
            r.uc,
            bold(r.cc / r.uc),
            r.cd,
            r.ud,
            bold(r.cd / r.ud),
            boldr(r.ratio)
        ));
    }
    let n = rows.len().max(1) as f64;
    let wins_c = rows.iter().filter(|r| r.cc / r.uc < 1.0).count();
    let wins_d = rows.iter().filter(|r| r.cd / r.ud < 1.0).count();
    let wins_r = rows.iter().filter(|r| r.ratio < 1.0).count();
    let (worst_ck_id, worst_ck) = rows
        .iter()
        .max_by(|a, b| a.ck_tax.partial_cmp(&b.ck_tax).unwrap())
        .map(|r| (r.id.as_str(), r.ck_tax))
        .unwrap_or(("-", 0.0));
    s.push_str(&format!(
        "\n**mean C/us comp {:.2}, decomp {:.2} | mean ratio {:.3} | worst ratio {:.3} ({}) | \
         we beat C: {} comp, {} decomp, {} ratio | board {:.0}s**\n\n\
         **GATE 4 parity: `us comp` measured with checksum OFF, matching `zstd -b`. \
         Checksum-on tax: mean +{:.1}%, worst +{:.1}% ({}). \
         `runallfour.rs` boards carry this tax; these do not.**\n",
        rows.iter().map(|r| r.cc / r.uc).sum::<f64>() / n,
        rows.iter().map(|r| r.cd / r.ud).sum::<f64>() / n,
        rows.iter().map(|r| r.ratio).sum::<f64>() / n,
        rows.last().map(|r| r.ratio).unwrap_or(0.0),
        rows.last().map(|r| r.id.as_str()).unwrap_or("-"),
        wins_c,
        wins_d,
        wins_r,
        secs,
        rows.iter().map(|r| r.ck_tax).sum::<f64>() / n,
        worst_ck,
        worst_ck_id,
    ));
    s
}

/// Replace the table under `### L<lvl> ...` with `body`, keeping any prose that
/// follows the summary line (the verdict block).
fn patch(doc: &str, lvl: i32, body: &str) -> String {
    let head = format!("### L{lvl} ");
    let Some(hs) = doc.find(&head) else {
        return doc.to_string();
    };
    let after = hs + doc[hs..].find('\n').unwrap_or(0) + 1;
    let rest = &doc[after..];
    let end = rest
        .find("\n### ")
        .into_iter()
        .chain(rest.find("\n## "))
        .min()
        .map(|e| after + e)
        .unwrap_or(doc.len());
    // keep everything from the first verdict line ("Gate ") onward
    let tail = doc[after..end]
        .find("\nGate ")
        .map(|i| doc[after + i..end].to_string())
        .unwrap_or_default();
    format!("{}\n{}{}\n{}", &doc[..after], body, tail, &doc[end..])
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gate: Option<u32> = args.get(1).and_then(|s| s.parse().ok());
    let verdict = args.get(2).cloned();
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let t0 = Instant::now();
    let mut doc = std::fs::read_to_string(DOC).expect("read plan");
    println!("RUN ALL FOUR (ADDENDUM) — L1, L3, L19, L22 x 18 corpora, full encode+decode");
    println!("  us arm: checksum OFF (parity with `zstd -b`)  |  C arm: zstd --ultra -b<lvl> -i1 -T1");
    for &(lvl, cap) in LEVELS {
        print!("  L{lvl} ({} MiB) ", cap / 1048576);
        let _ = std::io::stdout().flush();
        let t = Instant::now();
        let rows = board(lvl, cap, zstd);
        let secs = t.elapsed().as_secs_f64();
        println!(" {} corpora in {secs:.0}s", rows.len());
        doc = patch(&doc, lvl, &render(&rows, secs));
    }
    if let (Some(g), Some(v)) = (gate, verdict) {
        for &(lvl, _) in LEVELS {
            let head = format!("### L{lvl} ");
            if let Some(hs) = doc.find(&head) {
                let rest = &doc[hs..];
                let end = rest[1..]
                    .find("\n### ")
                    .map(|e| hs + 1 + e)
                    .unwrap_or(doc.len());
                let line = format!("\nGate {g} @ L{lvl} = **{v}**\n");
                doc = format!("{}{}{}", &doc[..end], line, &doc[end..]);
            }
        }
        println!("  appended Gate {g} verdict: {v}");
    }
    std::fs::write(DOC, doc).expect("write plan");
    println!(
        "\nDONE in {:.0}s — {} updated",
        t0.elapsed().as_secs_f64(),
        DOC
    );
}
