//! Function-level DecSeq anatomy -- the decode-side twin of `mfanat.rs`.
//!
//! WHY THIS INSTRUMENT IS SHAPED DIFFERENTLY FROM `mfanat.rs`.
//! MatchFind is a per-BLOCK call, so an `Instant` RAII guard around it is free
//! relative to the work it measures. DecSeq is one flat loop over ~30M
//! sequences whose body is ~10-20ns; an `Instant` pair is ~20-25ns on this box,
//! so a per-sequence guard would COST MORE THAN THE THING IT MEASURES and would
//! report the instrument, not the codec. So DecSeq is resolved in two halves:
//!
//!   (a) TIME, per block: four guards partition DecSeq into Header / Tables /
//!       Loop / Tail. That is 4 guards per 128 KiB block -- a tax this prints
//!       and prices rather than assumes (`--tax` mode).
//!   (b) COUNTS, per sequence: the loop's interior is deterministic, so every
//!       function in it is reported as calls/MiB and bytes/MiB from the census
//!       counters -- no clock involved, nothing to be noisy.
//!
//! Usage:
//!   dsanat            -- (a) per-level board + (b) per-corpus board at L3
//!   dsanat tax        -- price the four guards against an unguarded DecSeq
use rusty_zstd::ProfStage as S;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .ok()
}

#[derive(Default, Clone, Copy)]
struct Row {
    seq: f64,
    hdr: f64,
    tab: f64,
    lp: f64,
    tail: f64,
    dec: f64,
    lits: f64,
    ck: f64,
    mib: f64,
    blocks: u64,
    nseq: u64,
    bands: [u64; 8],
    band_b: [u64; 8],
    tiers: (u64, u64, u64, u64),
    /// Worst-over-best DecSeq gap across the kept iterations, in percent.
    spread: f64,
}

/// Decodes to warm the allocator and the caches before anything is believed.
const WARMUP: usize = 2;
/// Timed decodes kept after warmup; the BEST (lowest DecSeq) is the row, and the
/// best/worst gap is this row's own null arm.
const BEST_OF: usize = 5;

/// One corpus, fully measured. `cap` bytes of `id` at level `lvl`.
///
/// Best-of-N over a warmed process. The first version of this instrument took a
/// SINGLE decode per corpus and reported table (a) 2.8x above table (b) on
/// identical inputs -- the gap was entirely cold-start (first-touch page faults
/// on the output `Vec`, a cold allocator, cold caches), not the codec. Shares
/// survived it because they are ratios within one run; absolute ms/MiB did not.
fn measure(id: &str, lvl: i32, cap: usize, want_nseq: bool) -> Option<Row> {
    let f = load(id)?;
    let s = &f[..f.len().min(cap)];
    let z = rusty_zstd::compress(s, lvl).ok()?;

    let mut best: Option<Row> = None;
    let mut worst_seq = 0f64;
    for it in 0..(WARMUP + BEST_OF) {
        // The encoder ran too; reset AFTER it so only decode is in the buckets.
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_dec_bands();
        let _ = rusty_zstd::take_dec_copies();

        let out = rusty_zstd::decompress(&z).ok()?;
        assert_eq!(out.len(), s.len(), "{id}: decode did not round-trip");
        let r = snapshot(s);
        if it < WARMUP {
            continue;
        }
        if r.seq > worst_seq {
            worst_seq = r.seq;
        }
        if best.as_ref().is_none_or(|b| r.seq < b.seq) {
            best = Some(r);
        }
    }
    let mut r = best?;
    // Same-arm spread on THIS row -- the board carries its own null arm.
    r.spread = if r.seq > 0.0 {
        100.0 * (worst_seq - r.seq) / r.seq
    } else {
        0.0
    };
    if want_nseq {
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::compress(s, lvl).ok()?;
        r.nseq = rusty_zstd::prof_encode_counts().seqs;
    }
    Some(r)
}

/// Read every counter for the decode that just ran.
fn snapshot(s: &[u8]) -> Row {
    let (bands, band_b) = rusty_zstd::take_dec_bands();
    Row {
        seq: rusty_zstd::prof_stage_ns(S::DecodeSeq) as f64,
        hdr: rusty_zstd::prof_stage_ns(S::DecSeqHeader) as f64,
        tab: rusty_zstd::prof_stage_ns(S::DecSeqTables) as f64,
        lp: rusty_zstd::prof_stage_ns(S::DecSeqLoop) as f64,
        tail: rusty_zstd::prof_stage_ns(S::DecSeqTail) as f64,
        dec: rusty_zstd::prof_stage_ns(S::DecodeTotal) as f64,
        lits: rusty_zstd::prof_stage_ns(S::DecodeLiterals) as f64,
        ck: rusty_zstd::prof_stage_ns(S::DecodeChecksum) as f64,
        mib: s.len() as f64 / 1_048_576.0,
        blocks: rusty_zstd::prof_stage_calls(S::DecSeqLoop),
        nseq: 0,
        bands,
        band_b,
        tiers: rusty_zstd::take_dec_copies(),
        spread: 0.0,
    }
}

fn add(a: &mut Row, b: &Row) {
    a.seq += b.seq;
    a.hdr += b.hdr;
    a.tab += b.tab;
    a.lp += b.lp;
    a.tail += b.tail;
    a.dec += b.dec;
    a.lits += b.lits;
    a.ck += b.ck;
    a.mib += b.mib;
    a.blocks += b.blocks;
    a.nseq += b.nseq;
    for i in 0..8 {
        a.bands[i] += b.bands[i];
        a.band_b[i] += b.band_b[i];
    }
    a.tiers.0 += b.tiers.0;
    a.tiers.1 += b.tiers.1;
    a.tiers.2 += b.tiers.2;
    a.tiers.3 += b.tiers.3;
    if b.spread > a.spread {
        a.spread = b.spread;
    }
}

/// ms per input MiB.
fn per_mib(ns: f64, mib: f64) -> f64 {
    if mib == 0.0 {
        0.0
    } else {
        ns / mib / 1_000_000.0
    }
}
fn pct(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        100.0 * a / b
    }
}

fn main() {
    let cap: usize = 8 << 20;
    if std::env::args().nth(1).as_deref() == Some("tax") {
        return tax(cap);
    }

    // ---- (a) per level: DecSeq's absolute cost and share, and its split ----
    println!("(a) DecSeq by LEVEL -- 18-corpus 8 MiB board, profile build, ms per input MiB\n");
    println!("| level | strategy | DecSeq ms/MiB | DecSeq % of decode | Header | Tables | Loop | Tail | decode ms/MiB | worst spread |");
    println!("| ----: | -------- | ------------: | -----------------: | -----: | -----: | ---: | ---: | ------------: | -----------: |");
    for lvl in [1i32, 3, 5, 7, 9, 12, 13, 15, 16, 19, 22] {
        let p = rusty_zstd::compression_params(lvl, None).unwrap();
        let mut t = Row::default();
        for id in IDS {
            if let Some(r) = measure(id, lvl, cap, false) {
                add(&mut t, &r);
            }
        }
        println!(
            "| L{lvl} | {:?} | {:.2} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.2} | {:.1}% |",
            p.strategy,
            per_mib(t.seq, t.mib),
            pct(t.seq, t.dec),
            pct(t.hdr, t.seq),
            pct(t.tab, t.seq),
            pct(t.lp, t.seq),
            pct(t.tail, t.seq),
            per_mib(t.dec, t.mib),
            t.spread,
        );
    }

    // ---- (b) per corpus at the shipping default ----
    for lvl in [1i32, 3] {
        println!("\n(b) DecSeq by CORPUS @ L{lvl} -- ms/MiB and the four sub-phases as % of DecSeq\n");
        println!("| corpus | DecSeq ms/MiB | % of decode | Header | Tables | Loop | Tail | ns/seq | seqs/MiB | spread |");
        println!("| ------ | ------------: | ----------: | -----: | -----: | ---: | ---: | -----: | -------: | -----: |");
        let mut rows: Vec<(String, Row)> = Vec::new();
        for id in IDS {
            if let Some(r) = measure(id, lvl, cap, true) {
                rows.push(((*id).to_string(), r));
            }
        }
        rows.sort_by(|a, b| pct(b.1.seq, b.1.dec).partial_cmp(&pct(a.1.seq, a.1.dec)).unwrap());
        let mut t = Row::default();
        for (id, r) in &rows {
            add(&mut t, r);
            println!(
                "| {id} | {:.2} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.0} | {:.1}% |",
                per_mib(r.seq, r.mib),
                pct(r.seq, r.dec),
                pct(r.hdr, r.seq),
                pct(r.tab, r.seq),
                pct(r.lp, r.seq),
                pct(r.tail, r.seq),
                if r.nseq > 0 { r.lp / r.nseq as f64 } else { 0.0 },
                if r.mib > 0.0 { r.nseq as f64 / r.mib } else { 0.0 },
                r.spread,
            );
        }
        println!(
            "| **TOTAL** | **{:.2}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.1}** | **{:.0}** | **{:.1}%** |",
            per_mib(t.seq, t.mib),
            pct(t.seq, t.dec),
            pct(t.hdr, t.seq),
            pct(t.tab, t.seq),
            pct(t.lp, t.seq),
            pct(t.tail, t.seq),
            if t.nseq > 0 { t.lp / t.nseq as f64 } else { 0.0 },
            if t.mib > 0.0 { t.nseq as f64 / t.mib } else { 0.0 },
            t.spread,
        );

        // ---- (c) the loop interior, by COUNT (no clock) ----
        println!("\n(c) The DecSeqLoop interior @ L{lvl} -- deterministic counts, whole board\n");
        let n = t.nseq as f64;
        let blocks = t.blocks.max(1) as f64;
        println!("| function | unit | calls (board) | calls/MiB | per sequence |");
        println!("| -------- | ---- | ------------: | --------: | -----------: |");
        let mib = t.mib.max(1.0);
        let row = |name: &str, unit: &str, calls: f64| {
            println!(
                "| `{name}` | {unit} | {:.0} | {:.0} | {:.2} |",
                calls,
                calls / mib,
                if n > 0.0 { calls / n } else { 0.0 }
            );
        };
        row("seq_table (LL/OF/ML)", "per block x3", blocks * 3.0);
        row("BitRev::new + init_state x3", "per block x4", blocks * 4.0);
        row("BitRev::reload", "per seq x2", n * 2.0);
        row("FseTable::entry", "per seq x3", n * 3.0);
        row("BitRev::read_bits", "per seq x3", n * 3.0);
        row("copy_literals", "per seq", n);
        row("resolve_offset", "per seq", n);
        row("copy_match", "per seq", n);
        row("FseTable::advance", "per seq x3", (n - blocks).max(0.0) * 3.0);

        let bt: u64 = t.bands.iter().sum();
        let bb: u64 = t.band_b.iter().sum();
        const BN: [&str; 8] = [
            "offset==1 splat",
            "32B tier (len>16)",
            "16B tier",
            "extend_from_within",
            "overlapping chunked",
            "32B tier (len<=16)",
            "64B tier",
            "(unused)",
        ];
        println!("\n**`copy_match` route census @ L{lvl}** -- which band each match copy took:\n");
        println!("| band | calls | % calls | bytes | % bytes | mean len |");
        println!("| ---- | ----: | ------: | ----: | ------: | -------: |");
        for i in 0..8 {
            println!(
                "| {} | {} | {:.1} | {} | {:.1} | {:.1} |",
                BN[i],
                t.bands[i],
                pct(t.bands[i] as f64, bt as f64),
                t.band_b[i],
                pct(t.band_b[i] as f64, bb as f64),
                if t.bands[i] > 0 { t.band_b[i] as f64 / t.bands[i] as f64 } else { 0.0 }
            );
        }
        let (l32, m32, l16, m16) = t.tiers;
        println!(
            "\n`copy_literals` tiers: 16B {l16}, 32B {l32}, memcpy fallthrough {} of {} calls ({:.1}% tiered)",
            (t.nseq).saturating_sub(l16 + l32),
            t.nseq,
            pct((l16 + l32) as f64, t.nseq as f64),
        );
        println!("`copy_match` wide tiers: 16B {m16}, 32B {m32}");
    }
}

/// Price the four guards: compare instrumented DecSeq against the same decode
/// with the sub-phase guards disabled at the source level is not possible in
/// one binary, so this reports the guards' own call count and the implied cost
/// at this box's measured `Instant` pair cost, plus the residual
/// DecSeq - (Header+Tables+Loop+Tail), which is where the tax lands.
fn tax(cap: usize) {
    // Cost of one Instant pair on this box, measured, not assumed.
    let n = 2_000_000u32;
    let t0 = std::time::Instant::now();
    let mut acc = 0u64;
    for _ in 0..n {
        let g = std::time::Instant::now();
        acc = acc.wrapping_add(g.elapsed().as_nanos() as u64);
    }
    let pair_ns = t0.elapsed().as_nanos() as f64 / n as f64;
    println!("Instant pair on this box: {pair_ns:.1} ns  [{acc}]\n");

    println!("| corpus | blocks | guards | guard ns (implied) | DecSeq ns | tax % | residual ns | residual % |");
    println!("| ------ | -----: | -----: | -----------------: | --------: | ----: | ----------: | ---------: |");
    for id in IDS {
        let Some(r) = measure(id, 3, cap, false) else { continue };
        let guards = r.blocks * 4;
        let implied = guards as f64 * pair_ns;
        let resid = r.seq - (r.hdr + r.tab + r.lp + r.tail);
        println!(
            "| {id} | {} | {} | {:.0} | {:.0} | {:.3} | {:.0} | {:.2} |",
            r.blocks,
            guards,
            implied,
            r.seq,
            pct(implied, r.seq),
            resid,
            pct(resid, r.seq),
        );
    }
}
