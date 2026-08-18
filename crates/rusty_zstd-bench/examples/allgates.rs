//! ALL GATES — ONE command that re-tests every gate, constant and dispatch in
//! the codec, so a verdict taken today survives the next campaign's edits.
//!
//!   cargo run --release -p rusty_zstd-bench --example allgates
//!   cargo run --release -p rusty_zstd-bench --example allgates -- 262144
//!
//! The optional arg is the per-corpus prefix in bytes for L1/L3 (L19/L22 use a
//! quarter of it). Default 2 MiB.
//!
//! **The prefix is not a free knob.** Blocks are 128 KiB, and several gates key
//! off a RUN of blocks (`fast_lazy` needs 4 consecutive qualifying blocks,
//! `raw_probe` re-probes every 16, `rep_yield`/`next_long_yield`/`tag_yield` are
//! all carried from the PREVIOUS block). A prefix that yields one or two blocks
//! makes every one of those structurally inert and they report DEAD for a reason
//! that has nothing to do with the gate. The header prints the block count; if it
//! is under ~16, treat every DEAD verdict as unproven.
//!
//! # What it does, and why in this order
//!
//! **1. The ARM sweep — the ON DECK protocol, mechanized.**
//! Every gate campaign opens with the same question: *CONFIRM GATE ISN'T DEAD BY
//! VALIDATING DEFAULT DIFFERS FROM VALUE SET.* Doing that by hand is how
//! `RZSTD_TAG` shipped a null A/B for a whole campaign (gg-matchfind.md §4.10) —
//! the "off" arm was already off, so the comparison measured nothing.
//!
//! Here it is deterministic and automatic. For each arm we take a pristine
//! BASELINE size vector (18 corpora x 4 levels), then set the arm to each value
//! in its set and re-measure. The arm's DEPLOYED value is *discovered*, not
//! assumed: it is whichever value reproduces the baseline byte counts. That
//! yields four verdicts:
//!
//!   LIVE   — some value reproduces the baseline, some value does not.
//!            The arm is wired and its default is known. This is a real gate.
//!   SZ-DEAD — every value reproduces the baseline. The arm cannot change the
//!            OUTPUT at any of the four levels. Read it two ways and pick by
//!            what the arm is FOR: for a size dispatch (rep1_mode, incomp_skip,
//!            fast_lazy) it is a null A/B and the gate is dead; for a
//!            byte-identical speed capability (dfast_spec, pipe, tag, litpush,
//!            payload_reserve) it is the IDENTITY PROOF that capability owes,
//!            and the speed question is still open — measure it on the clock.
//!   DRIFT  — NO value reproduces the baseline. The unresolved sentinel state is
//!            a third behaviour, or the arm latches. Always a defect.
//!   STUCK  — LIVE/DEAD determined, but the baseline could not be restored
//!            afterwards. The baseline is re-taken and the sweep continues, but
//!            the arm has no setter that reaches its shipped state.
//!
//! Sizes are DETERMINISTIC, so none of this needs pinning, a quiet box, or a
//! noise floor. It is valid on a busy machine and it is valid in CI.
//!
//! **2. The CALLER sweep.** Nine of the twenty gates in `gg-Addendum.md` are
//! caller-supplied options that `compress(src, lvl)` never sets — MT, dict,
//! prefix, checksum, LDM, rsyncable, target-cblock, prime-only, window-max,
//! force-ignore-checksum, skippable frames. A corpus board CANNOT see them. Each
//! is driven here through its correct entry point and classified WIRED (setting
//! it changes the output or the behaviour) or UNWIRED (it does not — a defect).
//!
//! **3. The knob census.** Counted from source, because the count in
//! `m7-anatomy.md` §3 Addendum was stale by ~20 in one direction and 1 in the
//! other. Never hand-count this again.
use rusty_zstd::{AdvancedOptions, CompressOptions, DecompressOptions};
use std::io::Write;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];
const LEVELS: &[i32] = &[1, 3, 19, 22];

fn load(cap: usize) -> Vec<(&'static str, Vec<u8>)> {
    IDS.iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    (*id, f[..n].to_vec())
                })
        })
        .collect()
}

/// Encode with checksum OFF — work-count parity with `zstd -b`, and 4 bytes of
/// trailer cannot mask a 4-byte payload difference.
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

/// The deterministic fingerprint of the whole codec: one size per corpus per
/// level. Two fingerprints are equal iff no routed decision changed anywhere.
fn fingerprint(srcs: &[(&'static str, Vec<u8>)], hi_cap: usize) -> Vec<usize> {
    let mut v = Vec::with_capacity(srcs.len() * LEVELS.len());
    for &lvl in LEVELS {
        let cap = if lvl >= 13 { hi_cap } else { usize::MAX };
        for (_, s) in srcs {
            let s = &s[..s.len().min(cap)];
            v.push(enc(s, lvl).len());
        }
    }
    v
}

/// Which (corpus, level) cells moved between two fingerprints.
fn diff_cells(a: &[usize], b: &[usize], srcs: &[(&'static str, Vec<u8>)]) -> Vec<String> {
    let n = srcs.len();
    let mut out = Vec::new();
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        if x != y {
            let lvl = LEVELS[i / n];
            let id = srcs[i % n].0;
            let d = *y as i64 - *x as i64;
            out.push(format!("L{lvl}:{id}{d:+}"));
        }
    }
    out
}

struct Arm {
    name: &'static str,
    /// (label, setter). The FIRST entry should be the believed-deployed value.
    vals: Vec<(&'static str, Box<dyn Fn()>)>,
}

fn b(f: impl Fn() + 'static) -> Box<dyn Fn()> {
    Box::new(f)
}

fn encode_arms() -> Vec<Arm> {
    use rusty_zstd::*;
    vec![
        Arm { name: "fast_lazy",       vals: vec![("on", b(|| set_fast_lazy_arm(true))), ("off", b(|| set_fast_lazy_arm(false)))] },
        Arm { name: "lazy_fill",       vals: vec![("on", b(|| set_lazy_fill_arm(true))), ("off", b(|| set_lazy_fill_arm(false)))] },
        Arm { name: "rep1_mode",       vals: vec![("dispatch", b(|| set_rep1_mode(None))), ("on", b(|| set_rep1_mode(Some(true)))), ("off", b(|| set_rep1_mode(Some(false))))] },
        Arm { name: "step0",           vals: vec![("2", b(|| set_step0_arm(2))), ("1", b(|| set_step0_arm(1))), ("3", b(|| set_step0_arm(3)))] },
        Arm { name: "pipe_rep1",       vals: vec![("on", b(|| set_pipe_rep1_arm(true))), ("off", b(|| set_pipe_rep1_arm(false)))] },
        Arm { name: "pipe",            vals: vec![("on", b(|| set_pipe_arm(true))), ("off", b(|| set_pipe_arm(false)))] },
        Arm { name: "huff_fast",       vals: vec![("on", b(|| set_huff_fast_arm(true))), ("off", b(|| set_huff_fast_arm(false)))] },
        Arm { name: "payload_reserve", vals: vec![("on", b(|| set_payload_arm(true))), ("off", b(|| set_payload_arm(false)))] },
        Arm { name: "litpush_hoist",   vals: vec![("on", b(|| set_litpush_hoist_arm(true))), ("off", b(|| set_litpush_hoist_arm(false)))] },
        Arm { name: "litpush",         vals: vec![("on", b(|| set_litpush_arm(true))), ("off", b(|| set_litpush_arm(false)))] },
        Arm { name: "dfast_step",      vals: vec![("dispatch", b(|| set_dfast_step_arm(0))), ("1", b(|| set_dfast_step_arm(1))), ("2", b(|| set_dfast_step_arm(2)))] },
        Arm { name: "dfast_spec_min",  vals: vec![("0.70", b(|| set_dfast_spec_min_arm(0.70))), ("0.0", b(|| set_dfast_spec_min_arm(0.0))), ("2.0", b(|| set_dfast_spec_min_arm(2.0)))] },
        Arm { name: "dfast_pipe",      vals: vec![("on", b(|| set_dfast_pipe_arm(true))), ("off", b(|| set_dfast_pipe_arm(false)))] },
        Arm { name: "search_log_d",    vals: vec![("0", b(|| set_search_log_delta(0))), ("-1", b(|| set_search_log_delta(-1))), ("+1", b(|| set_search_log_delta(1)))] },
        Arm { name: "opt_lit",         vals: vec![("auto", b(|| set_opt_lit_arm(u32::MAX))), ("6", b(|| set_opt_lit_arm(6))), ("9", b(|| set_opt_lit_arm(9)))] },
        Arm { name: "opt_rep",         vals: vec![("on", b(|| set_opt_rep_arm(true))), ("off", b(|| set_opt_rep_arm(false)))] },
        Arm { name: "dfast_spec",      vals: vec![("on", b(|| set_dfast_spec_arm(true))), ("off", b(|| set_dfast_spec_arm(false)))] },
        Arm { name: "fast_spec",       vals: vec![("on", b(|| set_fast_spec_arm(true))), ("off", b(|| set_fast_spec_arm(false)))] },
        Arm { name: "bt_spec",         vals: vec![("on", b(|| set_bt_spec_arm(true))), ("off", b(|| set_bt_spec_arm(false)))] },
        Arm { name: "next_long",       vals: vec![("on", b(|| set_next_long_arm(true))), ("off", b(|| set_next_long_arm(false)))] },
        Arm { name: "pair_on",         vals: vec![("on", b(|| set_pair_on_arm(true))), ("off", b(|| set_pair_on_arm(false)))] },
        Arm { name: "tag",             vals: vec![("on", b(|| set_tag_arm(true))), ("off", b(|| set_tag_arm(false)))] },
        Arm { name: "tag_alloc",       vals: vec![("on", b(|| set_tag_alloc_arm(true))), ("off", b(|| set_tag_alloc_arm(false)))] },
        Arm { name: "pair_hi",         vals: vec![("1.0", b(|| set_pair_hi_arm(1.0))), ("0.0", b(|| set_pair_hi_arm(0.0))), ("9.0", b(|| set_pair_hi_arm(9.0)))] },
        Arm { name: "pair_gain",       vals: vec![("0.20", b(|| set_pair_gain_arm(0.20))), ("0.0", b(|| set_pair_gain_arm(0.0))), ("1.0", b(|| set_pair_gain_arm(1.0)))] },
        Arm { name: "incomp_skip",     vals: vec![("level", b(|| set_incomp_skip_arm(None))), ("on", b(|| set_incomp_skip_arm(Some(true)))), ("off", b(|| set_incomp_skip_arm(Some(false))))] },
        Arm { name: "strategy",        vals: vec![("level", b(|| set_strategy_arm(None))), ("Greedy", b(|| set_strategy_arm(Some(Strategy::Greedy))))] },
    ]
}

fn arm_sweep(srcs: &[(&'static str, Vec<u8>)], hi_cap: usize) -> (usize, usize, usize, usize) {
    println!("\n================ 1. ARM SWEEP — every encode arm, all 4 levels ================");
    println!("{:<16} {:<8} {:<28} {}", "arm", "verdict", "deployed / values", "moved cells");
    println!("{}", "-".repeat(112));
    let mut base = fingerprint(srcs, hi_cap);
    let (mut live, mut dead, mut drift, mut stuck) = (0, 0, 0, 0);
    for arm in encode_arms() {
        let mut matching: Option<&'static str> = None;
        let mut moved: Vec<String> = Vec::new();
        let mut detail: Vec<String> = Vec::new();
        for (label, set) in &arm.vals {
            set();
            let fp = fingerprint(srcs, hi_cap);
            if fp == base {
                if matching.is_none() {
                    matching = Some(label);
                }
                detail.push(format!("{label}=base"));
            } else {
                let d = diff_cells(&base, &fp, srcs);
                detail.push(format!("{label}≠({} cells)", d.len()));
                if moved.is_empty() {
                    moved = d;
                }
            }
        }
        // restore to the value that reproduced the baseline
        let mut verdict = match (matching, moved.is_empty()) {
            (Some(_), true) => "SZ-DEAD",
            (Some(_), false) => "LIVE",
            (None, _) => "DRIFT",
        };
        if let Some(m) = matching {
            if let Some((_, set)) = arm.vals.iter().find(|(l, _)| *l == m) {
                set();
            }
        }
        let after = fingerprint(srcs, hi_cap);
        if after != base {
            verdict = "STUCK";
            base = after; // re-baseline so the rest of the sweep stays valid
            stuck += 1;
        } else {
            match verdict {
                "LIVE" => live += 1,
                "SZ-DEAD" => dead += 1,
                _ => drift += 1,
            }
        }
        let show: String = moved.iter().take(4).cloned().collect::<Vec<_>>().join(" ");
        let more = if moved.len() > 4 {
            format!(" +{}", moved.len() - 4)
        } else {
            String::new()
        };
        println!(
            "{:<16} {:<8} {:<28} {show}{more}",
            arm.name,
            verdict,
            format!("{} | {}", matching.unwrap_or("NONE"), detail.join(" ")),
        );
        let _ = std::io::stdout().flush();
    }
    println!("\n  LIVE {live} | SZ-DEAD {dead} | DRIFT {drift} | STUCK {stuck}");
    println!("  SZ-DEAD on a SIZE dispatch = a null A/B, the gate is dead.");
    println!("  SZ-DEAD on a byte-identical SPEED capability = the identity proof it owes;");
    println!("           the speed question is still open and belongs on the clock.");
    println!("  DRIFT / STUCK are always defects.");
    (live, dead, drift, stuck)
}

// ---------------------------------------------------------------------------

fn decode_sweep(srcs: &[(&'static str, Vec<u8>)]) {
    use rusty_zstd::*;
    println!("\n================ 2. DECODE ARM SWEEP — output must NEVER move ================");
    let arms: Vec<(&str, Vec<(&str, Box<dyn Fn()>)>)> = vec![
        ("seqcheck",  vec![("on", b(|| set_seqcheck_arm(true))),  ("off", b(|| set_seqcheck_arm(false)))]),
        ("lut",       vec![("on", b(|| set_lut_arm(true))),       ("off", b(|| set_lut_arm(false)))]),
        ("litcopy",   vec![("on", b(|| set_litcopy_arm(true))),   ("off", b(|| set_litcopy_arm(false)))]),
        ("matchcopy", vec![("on", b(|| set_matchcopy_arm(true))), ("off", b(|| set_matchcopy_arm(false)))]),
    ];
    // one frame per corpus per level, encoded once
    let mut frames = Vec::new();
    for &lvl in LEVELS {
        for (id, s) in srcs {
            let s = &s[..s.len().min(if lvl >= 13 { 1 << 17 } else { 1 << 19 })];
            frames.push((*id, lvl, s.to_vec(), enc(s, lvl)));
        }
    }
    for (name, vals) in arms {
        let mut bad = 0usize;
        for (label, set) in &vals {
            set();
            for (id, lvl, src, z) in &frames {
                let mut out = Vec::new();
                match rusty_zstd::decompress_into(&mut out, z) {
                    Ok(_) if out == *src => {}
                    _ => {
                        bad += 1;
                        println!("  !! {name}={label} MISDECODES {id} @ L{lvl}");
                    }
                }
            }
        }
        // leave every decode arm ON (the shipped setting for all four)
        vals[0].1();
        println!(
            "  {:<10} round-trip {} across {} frames x {} settings",
            name,
            if bad == 0 { "OK" } else { "FAILED" },
            frames.len(),
            vals.len()
        );
    }
    println!("  A decode arm may only change SPEED. Any row but OK is a correctness defect.");
}

// ---------------------------------------------------------------------------

fn caller_sweep(srcs: &[(&'static str, Vec<u8>)]) {
    println!("\n================ 3. CALLER GATE SWEEP — the nine a corpus board CANNOT see ================");
    println!("{:<22} {:<10} {}", "gate", "verdict", "evidence");
    println!("{}", "-".repeat(96));
    // a mid-sized, compressible corpus is enough: these are wiring checks
    let (_, src) = srcs
        .iter()
        .find(|(id, _)| *id == "samba")
        .or_else(|| srcs.first())
        .expect("no corpora");
    let src = &src[..src.len().min(1 << 20)];
    let lvl = 3;
    let base = enc(src, lvl);
    let row = |gate: &str, wired: bool, ev: String| {
        println!(
            "{:<22} {:<10} {ev}",
            gate,
            if wired { "WIRED" } else { "UNWIRED*" }
        );
    };

    // Gate 4 — checksum
    let ck_on = rusty_zstd::compress_with(src, CompressOptions { level: lvl, checksum: true }).unwrap();
    row(
        "4 checksum",
        ck_on.len() != base.len(),
        format!("{} vs {} bytes (Δ{})", ck_on.len(), base.len(), ck_on.len() as i64 - base.len() as i64),
    );

    // Gate 1 — nb_workers (MT). Must round-trip and must not corrupt.
    // `job_size = 0` means `4 * window`, which at L3 is 8 MiB -- larger than this
    // source, so MT would run ONE job and emit byte-identical output. That is a
    // null A/B dressed as a verdict; pin the job size so >1 job actually exists.
    let adv_mt = AdvancedOptions { nb_workers: 2, job_size: 128 * 1024, ..Default::default() };
    match rusty_zstd::compress_with_advanced(src, rusty_zstd::compression_params(lvl, Some(src.len() as u64)).unwrap(), false, None, &[], true, adv_mt) {
        Ok(z) => {
            let ok = rusty_zstd::decompress(&z).map(|d| d == src).unwrap_or(false);
            row("1 nb_workers=2", z != base, format!("{} vs {} bytes ({} jobs of 128 KiB), round-trip {}", z.len(), base.len(), src.len().div_ceil(128 << 10), if ok { "OK" } else { "FAILED" }));
        }
        Err(e) => row("1 nb_workers=2", false, format!("ERROR {e:?}")),
    }

    // Gates 2 + 10 — prefix (the dictionary path we can drive without a dict corpus)
    let (pre, tail) = src.split_at(src.len() / 2);
    let p_base = enc(tail, lvl);
    match rusty_zstd::compress_using_prefix(tail, pre, lvl) {
        Ok(z) => {
            let ok = rusty_zstd::decompress_using_prefix(&z, pre).map(|d| d == tail).unwrap_or(false);
            row("2/10 prefix", z.len() != p_base.len(), format!("{} vs {} bytes, round-trip {}", z.len(), p_base.len(), if ok { "OK" } else { "FAILED" }));
        }
        Err(e) => row("2/10 prefix", false, format!("ERROR {e:?}")),
    }

    // Gate 5 — RZSTD_BLOCK_KB (an UNCACHED env read, so it moves mid-process)
    std::env::set_var("RZSTD_BLOCK_KB", "32");
    let z32 = enc(src, lvl);
    std::env::remove_var("RZSTD_BLOCK_KB");
    let z_re = enc(src, lvl);
    row(
        "5 BLOCK_KB=32",
        z32.len() != base.len(),
        format!("{} vs {} bytes ({:+.3}%), restored {}", z32.len(), base.len(), (z32.len() as f64 / base.len() as f64 - 1.0) * 100.0, if z_re == base { "OK" } else { "FAILED" }),
    );

    // Gate 14 — target_cblock_size
    let adv_t = AdvancedOptions { target_cblock_size: 16384, ..Default::default() };
    let params = rusty_zstd::compression_params(lvl, Some(src.len() as u64)).unwrap();
    match rusty_zstd::compress_with_advanced(src, params, false, None, &[], true, adv_t) {
        Ok(z) => row("14 target_cblock", z.len() != base.len(), format!("{} vs {} bytes ({:+.3}%)", z.len(), base.len(), (z.len() as f64 / base.len() as f64 - 1.0) * 100.0)),
        Err(e) => row("14 target_cblock", false, format!("ERROR {e:?}")),
    }

    // Gate 15 — rsyncable
    let adv_r = AdvancedOptions { rsyncable: true, ..Default::default() };
    match rusty_zstd::compress_with_advanced(src, params, false, None, &[], true, adv_r) {
        Ok(z) => {
            let ok = rusty_zstd::decompress(&z).map(|d| d == src).unwrap_or(false);
            row("15 rsyncable", z.len() != base.len(), format!("{} vs {} bytes ({:+.3}%), round-trip {}", z.len(), base.len(), (z.len() as f64 / base.len() as f64 - 1.0) * 100.0, if ok { "OK" } else { "FAILED" }));
        }
        Err(e) => row("15 rsyncable", false, format!("ERROR {e:?}")),
    }

    // Gate 16 — LDM
    let adv_l = AdvancedOptions { ldm: rusty_zstd::LdmParams::enabled(), ..Default::default() };
    match rusty_zstd::compress_with_advanced(src, params, false, None, &[], true, adv_l) {
        Ok(z) => {
            let ok = rusty_zstd::decompress(&z).map(|d| d == src).unwrap_or(false);
            row("16 ldm", z.len() != base.len(), format!("{} vs {} bytes ({:+.3}%), round-trip {}", z.len(), base.len(), (z.len() as f64 / base.len() as f64 - 1.0) * 100.0, if ok { "OK" } else { "FAILED" }));
        }
        Err(e) => row("16 ldm", false, format!("ERROR {e:?}")),
    }

    // Gate 9 — decoder window_max rejection
    let tiny = DecompressOptions { window_max: 1024, ..Default::default() };
    let rejected = rusty_zstd::decompress_with(&base, tiny).is_err();
    row("9 window_max=1KiB", rejected, format!("over-large-window frame {}", if rejected { "REJECTED (correct)" } else { "ACCEPTED — the cap does not bind" }));

    // Gate 20 — force_ignore_checksum: must CONSUME the 4 bytes, not verify them
    let mut corrupt = ck_on.clone();
    let n = corrupt.len();
    corrupt[n - 1] ^= 0xFF;
    let strict = rusty_zstd::decompress(&corrupt).is_err();
    let lax = rusty_zstd::decompress_with(&corrupt, DecompressOptions { force_ignore_checksum: true, ..Default::default() }).is_ok();
    row("20 force_ignore_ck", strict && lax, format!("corrupt trailer: strict {}, lax {}", if strict { "rejects" } else { "ACCEPTS — verification is not running" }, if lax { "accepts" } else { "REJECTS — the flag is not honoured" }));

    // Gate 11 — frame magic: a skippable frame must be consumed and ignored
    let mut skip = Vec::new();
    skip.extend_from_slice(&rusty_zstd::MAGIC_SKIPPABLE_MIN.to_le_bytes());
    skip.extend_from_slice(&8u32.to_le_bytes());
    skip.extend_from_slice(&[0u8; 8]);
    skip.extend_from_slice(&base);
    let ok = rusty_zstd::decompress(&skip).map(|d| d == src).unwrap_or(false);
    row("11 frame magic", ok, format!("skippable+zstd concatenation {}", if ok { "decodes to the payload (correct)" } else { "FAILED" }));

    // Gate 12 — empty frame
    let ez = enc(&[], lvl);
    let ok = rusty_zstd::decompress(&ez).map(|d| d.is_empty()).unwrap_or(false);
    row("12 empty frame", ok, format!("{} bytes, round-trips to empty {}", ez.len(), if ok { "OK" } else { "FAILED" }));

    println!("\n  *UNWIRED means setting the option changed nothing observable. For a");
    println!("   CALLER gate that is a DEFECT, not a verdict — a corpus board would then");
    println!("   report \"no change on 18 corpora\" for a knob that is simply not connected.");
}

// ---------------------------------------------------------------------------

fn knob_census() {
    println!("\n================ 4. KNOB CENSUS — counted from source, never quoted ================");
    let files = [
        "crates/rusty_zstd/src/encode.rs",
        "crates/rusty_zstd/src/compressed.rs",
        "crates/rusty_zstd/src/params.rs",
        "crates/rusty_zstd/src/simd.rs",
    ];
    let (mut switches, mut counters, mut oncelocks, mut setters) = (0, 0, 0, 0);
    let mut uncached: Vec<String> = Vec::new();
    for f in files {
        let Ok(s) = std::fs::read_to_string(f) else {
            println!("  (missing {f})");
            continue;
        };
        for line in s.lines() {
            let t = line.trim_start();
            // `[A-Z0-9_]*` — DIGITS INCLUDED. The first pass at this count used
            // `[A-Z_]*`, which missed `REP1_ENABLED_ARM` and shipped 11 for 12.
            let is_static = t.starts_with("static ") || t.starts_with("pub static ");
            if is_static && t.contains("Atomic") {
                if t.contains("_ARM") || t.contains("_ARM:") {
                    switches += 1;
                } else {
                    counters += 1;
                }
            }
            if t.contains("OnceLock<") && is_static {
                oncelocks += 1;
            }
            if t.starts_with("pub fn set_") {
                setters += 1;
            }
        }
        // an env read is UNCACHED when its function body has no `_ARM.store` or
        // `OnceLock` beside it — approximated by scanning each fn block
        for blk in s.split("\nfn ").skip(1) {
            let name = blk.split('(').next().unwrap_or("").trim().to_string();
            let body = blk.split("\n}").next().unwrap_or("");
            if body.contains("env::var")
                && !body.contains("OnceLock")
                && !body.contains(".store(")
            {
                for seg in body.split("env::var(\"").skip(1) {
                    if let Some(k) = seg.split('"').next() {
                        uncached.push(format!("{k} ({name})"));
                    }
                }
            }
        }
    }
    println!("  control arms (switches) ... {switches}");
    println!("  pure counters ............. {counters}   <- excluded from the knob count");
    println!("  OnceLocks ................. {oncelocks}");
    println!("  pub fn set_* setters ...... {setters}");
    println!("  UNCACHED env reads ........ {}", uncached.len());
    for u in &uncached {
        println!("      {u}");
    }
    println!("\n  m7-anatomy.md §3 Addendum records \"17 knobs: 12 arms, 3 OnceLocks, 2");
    println!("  uncached env reads\". If the numbers above disagree, the DOC is stale —");
    println!("  this count is the one taken from source. Re-record it, do not re-quote it.");
}

fn main() {
    let cap: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2 * 1024 * 1024);
    let hi_cap = (cap / 4).max(65536);
    let srcs = load(cap);
    println!("ALL GATES — {} corpora, levels {LEVELS:?}", srcs.len());
    println!(
        "  prefix {} KiB = {} blocks (L1/L3), {} KiB = {} blocks (L19/L22); encode checksum OFF",
        cap >> 10,
        cap.div_ceil(128 << 10),
        hi_cap >> 10,
        hi_cap.div_ceil(128 << 10)
    );
    if cap.div_ceil(128 << 10) < 16 {
        println!("  !! WARNING: under 16 blocks. Gates keyed on a RUN of blocks (fast_lazy needs 4,");
        println!("     raw_probe re-probes every 16) are structurally inert — their DEAD is UNPROVEN.");
    }
    println!("  every verdict below is DETERMINISTIC (compressed sizes) — valid on a busy box");
    let t0 = std::time::Instant::now();
    let (live, dead, drift, stuck) = arm_sweep(&srcs, hi_cap);
    decode_sweep(&srcs);
    caller_sweep(&srcs);
    knob_census();
    println!(
        "\nDONE in {:.0}s — arms: {live} LIVE, {dead} SZ-DEAD, {drift} DRIFT, {stuck} STUCK",
        t0.elapsed().as_secs_f64()
    );
    if drift + stuck > 0 {
        println!("NON-ZERO DRIFT/STUCK — an arm cannot be returned to its shipped state. Fix before banking any verdict.");
    }
}
