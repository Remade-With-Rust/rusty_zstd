//! DecSeqLoop ns/sequence, nothing else.
//!
//! Exists to answer ONE question: does the `dupladder` arm dispatch perturb the
//! loop it measures? Build this twice from identical source --
//! `--features profile` and `--features dupladder` -- and compare. Any gap is
//! the instrument, not the codec.
use rusty_zstd::ProfStage as S;

const IDS: &[&str] = &[
    "reymont", "dickens", "webster", "mr", "smallmsg-8m", "jsonlog-16m", "nci", "samba", "osdb",
    "xml", "mozilla", "ooffice", "sao",
];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .ok()
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let reps: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = 8 << 20;
    let build = if cfg!(feature = "dupladder") { "dupladder" } else { "profile" };
    println!("DecSeqLoop ns/seq @ L{lvl} -- build: {build}, best-of-{reps} after 2 warmup\n");
    println!("| corpus | seqs | loop ns/seq | DecLits ms/MiB | decode ms/MiB | spread |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");
    let (mut tl, mut tn) = (0f64, 0u64);
    let (mut tlit, mut tdec, mut tmib) = (0f64, 0f64, 0f64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress(s, lvl).unwrap();
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let nseq = rusty_zstd::prof_encode_counts().seqs;
        if nseq < 1000 {
            continue;
        }
        let (mut best, mut worst) = (f64::MAX, 0f64);
        let (mut blit, mut bdec) = (f64::MAX, f64::MAX);
        for it in 0..(2 + reps) {
            rusty_zstd::prof_reset();
            let out = rusty_zstd::decompress(&z).unwrap();
            assert_eq!(out.len(), s.len());
            if it < 2 {
                continue;
            }
            let ns = rusty_zstd::prof_stage_ns(S::DecSeqLoop) as f64;
            let li = rusty_zstd::prof_stage_ns(S::DecodeLiterals) as f64;
            let dt = rusty_zstd::prof_stage_ns(S::DecodeTotal) as f64;
            if li < blit {
                blit = li;
            }
            if dt < bdec {
                bdec = dt;
            }
            if ns < best {
                best = ns;
            }
            if ns > worst {
                worst = ns;
            }
        }
        let mib = s.len() as f64 / 1_048_576.0;
        tl += best;
        tn += nseq;
        tlit += blit;
        tdec += bdec;
        tmib += mib;
        println!(
            "| {id} | {nseq} | {:.2} | {:.3} | {:.3} | {:.1}% |",
            best / nseq as f64,
            blit / mib / 1e6,
            bdec / mib / 1e6,
            100.0 * (worst - best) / best
        );
    }
    println!(
        "| **board** | **{tn}** | **{:.2}** | **{:.3}** | **{:.3}** | |",
        tl / tn as f64,
        tlit / tmib / 1e6,
        tdec / tmib / 1e6
    );
}
