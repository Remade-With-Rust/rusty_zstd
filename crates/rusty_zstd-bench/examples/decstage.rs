//! Stage breakdown: one-shot decode vs streaming decode, SAME bytes.
//!
//! The copy census says streaming decode moves ~2.1 B per output byte, which
//! prices at 1-2% of decode -- while the measured gap is 1.4-2.0x. So the
//! copies are not the gap, and the question is which STAGE grows. That is the
//! stage profiler's job.
//!
//! Profiled build: scope guards are rdtsc pairs, so absolute numbers carry the
//! instrument's own tax. Read the RATIO between the two arms per stage, since
//! both arms pay the same per-scope cost for the same call counts.
use rusty_zstd::{Decompressor, ProfStage};

const STAGES: &[(ProfStage, &str)] = &[
    (ProfStage::DecodeTotal, "DecodeTotal"),
    (ProfStage::DecodeBlocks, "DecodeBlocks"),
    (ProfStage::DecodeLiterals, "DecodeLiterals"),
    (ProfStage::DecodeSeq, "DecodeSeq"),
    (ProfStage::DecodeChecksum, "DecodeChecksum"),
    (ProfStage::DecSeqHeader, "  DecSeqHeader"),
    (ProfStage::DecSeqTables, "  DecSeqTables"),
    (ProfStage::DecSeqLoop, "  DecSeqLoop"),
    (ProfStage::DecSeqTail, "  DecSeqTail"),
    (ProfStage::StreamInAcc, "StreamInAcc"),
    (ProfStage::StreamProgress, "StreamProgress"),
    (ProfStage::StreamOutCopy, "StreamOutCopy"),
    (ProfStage::StreamCompact, "StreamCompact"),
];

fn snap() -> Vec<(u64, u64)> {
    STAGES
        .iter()
        .map(|(s, _)| {
            (
                rusty_zstd::prof_stage_ns(*s),
                rusty_zstd::prof_stage_calls(*s),
            )
        })
        .collect()
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let id = std::env::args().nth(2).unwrap_or_else(|| "webster".into());
    let f = std::fs::read(format!("corpora/data/silesia/{id}")).expect("corpus");
    let src = &f[..f.len().min(32 << 20)];
    let z = rusty_zstd::compress(src, lvl).expect("compress");
    let chunk = 64 << 10;

    let mut out = Vec::with_capacity(src.len());
    rusty_zstd::prof_reset();
    let t = std::time::Instant::now();
    rusty_zstd::decompress_into(&mut out, &z).unwrap();
    let wall_a = t.elapsed().as_secs_f64();
    let a = snap();

    let mut buf = vec![0u8; 128 << 10];
    rusty_zstd::prof_reset();
    let t = std::time::Instant::now();
    let mut d = Decompressor::new();
    let (mut n, mut i) = (0usize, 0usize);
    while i < z.len() {
        let end = (i + chunk).min(z.len());
        let mut inp = &z[i..end];
        loop {
            let st = d.stream(inp, &mut buf, false).expect("s");
            n += st.output_produced;
            inp = &inp[st.input_consumed..];
            if inp.is_empty() || (st.input_consumed == 0 && st.output_produced == 0) {
                break;
            }
        }
        i = end;
    }
    loop {
        let st = d.stream(&[], &mut buf, true).expect("d");
        n += st.output_produced;
        if st.output_produced == 0 {
            break;
        }
    }
    let wall_b = t.elapsed().as_secs_f64();
    let b = snap();
    assert_eq!(n, src.len());

    println!(
        "{id} L{lvl}  {:.1} MiB -- stage ns and CALLS, one-shot vs streaming\n",
        src.len() as f64 / (1 << 20) as f64
    );
    println!(
        "{:<16}{:>13}{:>13}{:>8}   {:>11}{:>11}{:>8}",
        "stage", "oneshot ms", "stream ms", "x", "os calls", "st calls", "x"
    );
    for (k, (s, name)) in STAGES.iter().enumerate() {
        let _ = s;
        let (na, ca) = a[k];
        let (nb, cb) = b[k];
        if na == 0 && nb == 0 {
            continue;
        }
        println!(
            "{name:<16}{:>13.2}{:>13.2}{:>8.2}   {ca:>11}{cb:>11}{:>8.2}",
            na as f64 / 1e6,
            nb as f64 / 1e6,
            if na > 0 { nb as f64 / na as f64 } else { 0.0 },
            if ca > 0 { cb as f64 / ca as f64 } else { 0.0 }
        );
    }
    // Residue = wall MINUS the stages that were scoped. Both arms are measured
    // in the SAME profiled build, so the per-scope rdtsc tax is common to both
    // and the comparison is like-for-like; mixing a profiled stage total
    // against an unprofiled wall is not.
    let sa = (a[2].0 + a[3].0 + a[4].0) as f64 / 1e9;
    let sb = (b[2].0 + b[3].0 + b[4].0) as f64 / 1e9;
    println!(
        "\n{:<16}{:>13}{:>13}{:>8}",
        "", "oneshot ms", "stream ms", "x"
    );
    println!(
        "{:<16}{:>13.2}{:>13.2}{:>8.2}",
        "WALL",
        wall_a * 1e3,
        wall_b * 1e3,
        wall_b / wall_a
    );
    println!(
        "{:<16}{:>13.2}{:>13.2}{:>8.2}",
        "scoped stages",
        sa * 1e3,
        sb * 1e3,
        sb / sa
    );
    println!(
        "{:<16}{:>13.2}{:>13.2}{:>8.2}   <-- everything OUTSIDE the decode stages",
        "RESIDUE",
        (wall_a - sa) * 1e3,
        (wall_b - sb) * 1e3,
        if wall_a - sa > 0.0 {
            (wall_b - sb) / (wall_a - sa)
        } else {
            0.0
        }
    );
    println!(
        "residue share: one-shot {:.1}%, streaming {:.1}% of its own wall",
        (wall_a - sa) / wall_a * 100.0,
        (wall_b - sb) / wall_b * 100.0
    );
    println!(
        "\nA stage whose CALL count matches but whose ns grows is doing the same work slower\n\
              (locality, allocation, or a per-call cost). A stage whose CALLS grow is being\n\
              re-entered more often -- a structural difference in how the path is driven."
    );
}
