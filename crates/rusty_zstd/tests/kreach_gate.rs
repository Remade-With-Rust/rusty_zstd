//! STANDING GATE: every shipping dispatch site must ROUTE to its kernel.
//!
//! This is the gate whose absence let a real defect live for months. The AVX2
//! xxh64 kernel in this crate was reachable only from a free function whose
//! callers were one unit test and one benchmark; the encoder, decoder and
//! streaming API all took the scalar route, and the DECODE side ran the
//! kernel on 0% of its bytes. Every other gate passed the whole time --
//! byte-identity passes because the two paths agree BY DESIGN, the round-trip
//! passes, conformance passes, and an arm-toggle A/B reads FLAT, which looks
//! exactly like "this kernel does not help" and gets written down as a
//! refutation nobody revisits.
//!
//! A count is the only instrument that separates "the kernel does not help"
//! from "the arm is not wired to anything". So this test counts.
//!
//! ONE `#[test]` IN THIS FILE, DELIBERATELY. The census counters are
//! process-global; cargo runs the tests inside one binary in parallel, so a
//! second test here would interleave its compressions with this one's and
//! corrupt both counts. Separate test binaries are separate processes and do
//! not interfere.
//!
//! The gate SKIPS a slot whose ISA the host CPU does not have, rather than
//! failing it -- a runner without AVX2 is not a routing defect. What it must
//! never do is pass silently because nothing ran, so it also asserts that the
//! sites it does check were actually exercised.
#![cfg(feature = "profile")]

use rusty_zstd::kreach::{self, N_SLOTS, SLOT_NAMES};

/// Slots that need BMI2 on the host to be reachable.
const NEEDS_BMI2: [usize; 8] = [
    kreach::K_FIND_FAST,
    kreach::K_EMIT_FAST_SEQ,
    kreach::K_FSE_CTABLE,
    kreach::K_FSE_WEIGHTS,
    kreach::K_HUF_ENC,
    kreach::K_DEC_SEQ,
    kreach::K_HUF_DEC4X,
    kreach::K_HUF_DEC4X1,
];

/// Slots that need AVX2 (x86) or NEON (aarch64).
const NEEDS_VEC: [usize; 2] = [kreach::K_COUNT_EQ_WIDE, kreach::K_XXH_STRIPE];

fn have_bmi2() -> bool {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        std::is_x86_feature_detected!("bmi2") && std::is_x86_feature_detected!("lzcnt")
    }
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    {
        false
    }
}

fn have_vec() -> bool {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        std::is_x86_feature_detected!("avx2")
    }
    #[cfg(target_arch = "aarch64")]
    {
        true // NEON is baseline in ARMv8-A.
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        false
    }
}

/// Content that actually reaches the sites under test: long repeats so the
/// match finder runs its wide common-prefix arm, a skewed literal alphabet so
/// Huffman and FSE both build and code real tables, and enough bytes to clear
/// the xxh64 tile threshold several times over.
fn corpus() -> Vec<u8> {
    let mut v = Vec::with_capacity(4 << 20);
    let words: [&str; 8] = [
        "the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ", "dog ",
    ];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    while v.len() < (4 << 20) {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let pick = (x >> 33) as usize % words.len();
        v.extend_from_slice(words[pick].as_bytes());
        // Periodic long repeats: these are what drive matches past 64 bytes,
        // which is the only way the wide `count_eq_len` arm is entered at all.
        if (x >> 60) & 7 == 0 {
            let take = v.len().min(4096);
            let start = v.len() - take;
            v.extend_from_within(start..start + take);
        }
    }
    v
}

#[test]
fn every_dispatch_site_routes_to_its_kernel() {
    let src = corpus();
    // SELF-VERIFICATION. An assertion that has never fired is not evidence:
    // a detached tap and a perfectly-routed kernel both print 100%. Setting
    // RZSTD_KREACH_POISON=1 forces every arm with a knob onto its scalar
    // side, and this test MUST then fail. Run it that way after touching any
    // dispatch site -- see CONTRIBUTING/CHANGELOG for the one-liner.
    if std::env::var_os("RZSTD_KREACH_POISON").is_some() {
        eprintln!("POISON: arms forced scalar; this test is EXPECTED to fail");
        rusty_zstd::set_xxh_avx2_arm(false);
        rusty_zstd::set_seqloop_avx2_arm(false);
        rusty_zstd::set_eqlen_arm(3);
    }
    let bmi2 = have_bmi2();
    let vec = have_vec();
    eprintln!(
        "host ISA: bmi2={bmi2} vec(avx2/neon)={vec}, corpus {} B",
        src.len()
    );

    let mut enc = [(0u64, 0u64); N_SLOTS];
    let mut dec = [(0u64, 0u64); N_SLOTS];

    // ONE LEVEL PER MATCH-FINDER STRATEGY, and asserted to be so.
    //
    // This was `[1, 3, 9]` under a comment claiming it covered "the distinct
    // match-finder strategies". It resolves to Fast, DFast and Lazy2 -- three
    // of eight. Greedy (L5), Lazy (L7), BtLazy2 (L13), BtOpt (L16) and
    // BtUltra2 (L19) were never compressed here, so any kernel reached only
    // from `find_greedy`, `find_lazy`, `find_bt_lazy` or `find_opt` scored
    // (0 kernel, 0 scalar) -- and the loop below SKIPS a slot with `h + m ==
    // 0` as "not exercised on this side". A dispatch that never took its twin
    // in those finders would have passed this gate on silence, which is
    // exactly the failure the poison self-check exists to prevent elsewhere.
    //
    // The high levels take a smaller prefix: BtOpt and BtUltra2 are orders of
    // magnitude slower per byte, and reach is a RATIO -- it does not need the
    // full corpus to be measured, only enough traffic to be non-zero.
    const LEVELS: &[(i32, usize)] = &[
        (1, 4 << 20),
        (3, 4 << 20),
        (5, 4 << 20),
        (7, 4 << 20),
        (9, 4 << 20),
        (13, 2 << 20),
        (16, 1 << 20),
        (18, 1 << 20),
        (19, 1 << 20),
    ];
    {
        let mut seen: Vec<String> = LEVELS
            .iter()
            .filter_map(|&(l, _)| rusty_zstd::compression_params(l, None).ok())
            .map(|p| format!("{:?}", p.strategy))
            .collect();
        seen.sort();
        seen.dedup();
        const WANT: &[&str] = &[
            "Fast", "DFast", "Greedy", "Lazy", "Lazy2", "BtLazy2", "BtOpt", "BtUltra", "BtUltra2",
        ];
        let missing: Vec<&str> = WANT
            .iter()
            .copied()
            .filter(|w| !seen.iter().any(|x| x == w))
            .collect();
        assert!(
            missing.is_empty(),
            "kreach_gate LEVELS no longer cover every match finder: missing \
             {missing:?} (covered: {seen:?}). A kernel reached only from a \
             missing finder would score 0/0 and be SKIPPED, so this gate would \
             pass on silence."
        );
        eprintln!("strategies exercised: {}", seen.join(", "));
    }
    for &(lvl, cap) in LEVELS {
        let s = &src[..src.len().min(cap)];
        let _ = kreach::take();
        let z = rusty_zstd::compress(s, lvl).expect("compress");
        let e = kreach::take();
        let out = rusty_zstd::decompress(&z).expect("decompress");
        let d = kreach::take();
        assert_eq!(out, s, "L{lvl} roundtrip");
        for i in 0..N_SLOTS {
            enc[i].0 += e[i].0;
            enc[i].1 += e[i].1;
            dec[i].0 += d[i].0;
            dec[i].1 += d[i].1;
        }
    }

    let mut checked = 0usize;
    let mut failures = Vec::new();
    for (side, t) in [("encode", &enc), ("decode", &dec)] {
        for i in 0..N_SLOTS {
            let needed = if NEEDS_VEC.contains(&i) {
                vec
            } else if NEEDS_BMI2.contains(&i) {
                bmi2
            } else {
                true
            };
            let (h, m) = t[i];
            if h + m == 0 {
                continue; // not exercised on this side; other sides cover it
            }
            if !needed {
                eprintln!(
                    "  {side}/{}: SKIPPED, host lacks the ISA ({h} kernel, {m} scalar)",
                    SLOT_NAMES[i].0
                );
                continue;
            }
            checked += 1;
            let pct = 100.0 * h as f64 / (h + m) as f64;
            eprintln!(
                "  {side}/{:<22} {h:>12} kernel {m:>10} scalar  {pct:>7.2}%",
                SLOT_NAMES[i].0
            );
            if pct < 95.0 {
                failures.push(format!(
                    "{side}/{} routed only {pct:.2}% of calls to its kernel \
                     ({h} kernel, {m} scalar) -- the twin exists but the \
                     shipping path is not taking it",
                    SLOT_NAMES[i].0
                ));
            }
        }
    }

    // A gate that checks nothing must fail, not pass. This is the failure mode
    // that let the original defect survive: silence read as success.
    assert!(
        checked >= 6,
        "kernel-reach gate exercised only {checked} sites -- it is not \
         measuring what it claims. Either the corpus stopped reaching the \
         dispatch sites or the census taps were detached from them."
    );
    assert!(
        failures.is_empty(),
        "kernel routing regressed:\n{}",
        failures.join("\n")
    );
}
