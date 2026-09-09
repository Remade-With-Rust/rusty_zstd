//! KERNEL REACH CENSUS -- one line per shipping dispatch site, encode and decode.
//!
//! Deterministic: same numbers on any machine, at any load. No clock, no
//! pinning, no ABBA, no z-score. Anything under 100% on the arch you ship is
//! a finding; the goal bar for this crate is >=95% per site.
//!
//! Encode and decode are censused SEPARATELY, because "50% of encode and 0%
//! of decode" is exactly the shape a combined number hides.
//!
//! Usage: `cargo run --release -p rusty_zstd-bench --example kreach \
//!         --features rusty_zstd/profile -- [level]`
use rusty_zstd::kreach::{self, N_SLOTS, SLOT_NAMES};

const IDS: &[&str] = &[
    "zeros-32m",
    "text-32m",
    "incomp-32m",
    "versions-16m",
    "jsonlog-16m",
    "smallmsg-8m",
    "dickens",
    "mozilla",
    "samba",
    "webster",
    "x-ray",
    "osdb",
    "reymont",
    "nci",
    "xml",
    "sao",
];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .ok()
}

fn add(t: &mut [(u64, u64); N_SLOTS], s: [(u64, u64); N_SLOTS]) {
    for i in 0..N_SLOTS {
        t[i].0 += s[i].0;
        t[i].1 += s[i].1;
    }
}

fn report(title: &str, t: &[(u64, u64); N_SLOTS], want: &str) -> bool {
    println!("\n{title}");
    println!(
        "  {:<24}{:>16}{:>16}{:>10}  {}",
        "dispatch site", "kernel calls", "scalar calls", "reach", "verdict"
    );
    let mut all_ok = true;
    let mut any = false;
    for i in 0..N_SLOTS {
        let (side, name) = (SLOT_NAMES[i].1, SLOT_NAMES[i].0);
        if side != want && want != "all" {
            continue;
        }
        let (h, m) = t[i];
        if h + m == 0 {
            println!("  {name:<24}{:>16}{:>16}{:>10}  NOT EXERCISED", h, m, "-");
            continue;
        }
        any = true;
        let pct = 100.0 * h as f64 / (h + m) as f64;
        let ok = pct >= 95.0;
        all_ok &= ok;
        println!(
            "  {name:<24}{h:>16}{m:>16}{:>9.2}%  {}",
            pct,
            if ok { "OK" } else { "*** UNDER 95% ***" }
        );
    }
    if !any {
        println!("  (no site on this side was exercised)");
    }
    all_ok
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    // POISON MODE. An all-100% census is worth nothing until the instrument
    // has been shown to REPORT A MISS -- otherwise "100%" and "the counter is
    // not wired" print identically. `--poison` forces every arm this crate
    // exposes a knob for onto its scalar side; those slots MUST then fall and
    // the run MUST exit non-zero. A poisoned run that still reads 100% is a
    // broken tap, not a reachable kernel.
    let poison = std::env::args().any(|a| a == "--poison");
    if poison {
        println!("*** POISON MODE: arms forced scalar; slots with a knob MUST drop ***");
        rusty_zstd::set_xxh_avx2_arm(false);
        rusty_zstd::set_seqloop_avx2_arm(false);
        // Arm 3, not 1: arm 1 short-circuits ABOVE the wide dispatch and so
        // never exercises the scalar side of that tap. Arm 3 routes through
        // the same site a non-AVX2 CPU takes, which is the branch under test.
        rusty_zstd::set_eqlen_arm(3);
    } else {
        rusty_zstd::set_xxh_avx2_arm(true);
    }

    let mut enc = [(0u64, 0u64); N_SLOTS];
    let mut dec = [(0u64, 0u64); N_SLOTS];
    let mut mib = 0f64;
    let mut n = 0;

    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        n += 1;
        mib += src.len() as f64 / (1 << 20) as f64;

        // ENCODE arm, censused alone.
        let _ = kreach::take();
        let z = rusty_zstd::compress(src, lvl).expect("compress");
        add(&mut enc, kreach::take());

        // DECODE arm, censused alone. The separation is the whole point.
        let out = rusty_zstd::decompress(&z).expect("decompress");
        add(&mut dec, kreach::take());
        assert_eq!(out, src, "{id} roundtrip");
    }

    println!("KERNEL REACH CENSUS -- L{lvl}, {n} corpora, {mib:.1} MiB");
    println!("counts, not clocks: identical on any machine at any load");
    let e = report("ENCODE", &enc, "enc");
    let d = report("DECODE", &dec, "dec");
    let b = report("BOTH SIDES (checksum)", &enc, "both");
    let b2 = report("BOTH SIDES (checksum) -- decode arm", &dec, "both");

    println!(
        "\nVERDICT: encode {}  decode {}  checksum-enc {}  checksum-dec {}",
        if e { "PASS" } else { "FAIL" },
        if d { "PASS" } else { "FAIL" },
        if b { "PASS" } else { "FAIL" },
        if b2 { "PASS" } else { "FAIL" }
    );
    if poison {
        // Inverted expectation: the poisoned run must FAIL, and a PASS here
        // means the taps are not measuring what their labels claim.
        if e && d && b && b2 {
            println!("\nPOISON CHECK FAILED: every slot still reads >=95% with the arms");
            println!("forced scalar. The census is not wired to the dispatch it names.");
            std::process::exit(1);
        }
        println!("\nPOISON CHECK PASSED: forcing the arms scalar moved the census.");
        return;
    }
    if !(e && d && b && b2) {
        std::process::exit(1);
    }
}
