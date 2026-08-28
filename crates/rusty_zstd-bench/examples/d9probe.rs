//! D9 adjudication: does prefetching the match source pay?
//!
//! The brick is byte-identical AND work-identical, so per inline-execution.md
//! 13.2 it carries no deterministic counter -- a clock is the only instrument
//! available, and this box's whole-decode null arm is 10.88-16.74%. Two things
//! make the measurement resolvable anyway:
//!
//!  1. **Measure the STAGE, not the process.** `prof_stage_ns(DecSeqLoop)`
//!     isolates the loop D9 touches from literal decode, checksum and I/O.
//!  2. **ABBA + a real NULL arm.** Each round runs A-B-B-A; the null arm is the
//!     same arm against itself, run identically, and it prints FIRST so the
//!     floor is known before any delta is read.
//!
//! Reports paired win rate and z = (wins - N/2) / (0.5*sqrt(N)); |z| > 2 is a
//! verdict. Round-trip asserted on every single decode.
use rusty_zstd::ProfStage as S;
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn once(z: &[u8], expect: &[u8], on: bool) -> f64 {
    rusty_zstd::set_prefetch_arm(on);
    rusty_zstd::prof_reset();
    let out = rusty_zstd::decompress(z).unwrap();
    assert!(out == expect, "prefetch={on}: OUTPUT CHANGED -- D9 is not byte-identical");
    rusty_zstd::prof_stage_ns(S::DecSeqLoop) as f64
}
fn med(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n == 0 { return 0.0 }
    if n % 2 == 1 { v[n/2] } else { 0.5*(v[n/2-1]+v[n/2]) }
}
/// One ABBA round: returns (arm_a_ns, arm_b_ns) with order alternated by `flip`.
fn abba(z: &[u8], s: &[u8], a: bool, b: bool, flip: bool) -> (f64, f64) {
    if flip {
        let b1 = once(z, s, b); let a1 = once(z, s, a);
        let a2 = once(z, s, a); let b2 = once(z, s, b);
        (0.5*(a1+a2), 0.5*(b1+b2))
    } else {
        let a1 = once(z, s, a); let b1 = once(z, s, b);
        let b2 = once(z, s, b); let a2 = once(z, s, a);
        (0.5*(a1+a2), 0.5*(b1+b2))
    }
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let rounds: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(21);
    let cap = 8usize << 20;
    println!("D9 HISTORY PREFETCH @ L{lvl} -- DecSeqLoop stage ns/seq, ABBA, {rounds} rounds\n");
    println!("| corpus | seqs | null% | OFF ns/seq | ON ns/seq | delta | wins | z |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    let (mut toff, mut ton, mut tseq) = (0f64, 0f64, 0u64);
    let (mut gw, mut gn) = (0usize, 0usize);
    let mut worst_null = 0f64;
    let mut all_null: Vec<f64> = Vec::new();
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress(s, lvl).unwrap();
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let nseq = rusty_zstd::prof_encode_counts().seqs;
        if nseq < 1000 { continue }
        let n = nseq as f64;
        for _ in 0..3 { let _ = once(&z, s, true); }        // warm
        let (mut vo, mut vn, mut vnull) = (vec![], vec![], vec![]);
        let (mut wins, mut pairs) = (0usize, 0usize);
        for r in 0..rounds {
            // NULL arm: ON vs ON, identical code, same ABBA shape.
            let (n1, n2) = abba(&z, s, true, true, r % 2 == 1);
            vnull.push(100.0 * (n1 - n2).abs() / n1.max(n2));
            all_null.push(100.0 * (n1 - n2).abs() / n1.max(n2));
            // Real arm.
            let (off, on) = abba(&z, s, false, true, r % 2 == 1);
            vo.push(off / n); vn.push(on / n);
            if on < off { wins += 1 }
            pairs += 1;
        }
        let (mo, mn) = (med(&mut vo), med(&mut vn));
        let nullw = vnull.iter().cloned().fold(0.0f64, f64::max);
        worst_null = worst_null.max(nullw);
        let z_ = (wins as f64 - pairs as f64/2.0) / (0.5*(pairs as f64).sqrt());
        println!("| {id} | {nseq} | {nullw:.1} | {mo:.2} | {mn:.2} | {:+.1}% | {wins}/{pairs} | {z_:+.2} |",
            100.0*(mn-mo)/mo);
        toff += mo*n; ton += mn*n; tseq += nseq;
        gw += wins; gn += pairs;
    }
    let (a, b) = (toff/tseq as f64, ton/tseq as f64);
    let gz = (gw as f64 - gn as f64/2.0) / (0.5*(gn as f64).sqrt());
    println!("\n**BOARD: OFF {a:.2} ns/seq -> ON {b:.2} ns/seq = {:+.1}%  |  {gw}/{gn} pairs, z = {gz:+.2}**",
        100.0*(b-a)/a);
    // Report BOTH. The worst pair is the pessimistic statistic, and quoting it
    // alone overstated this box's floor ~7x: rowspeed.rs measured median 4.79%
    // against worst 20.24% on the same machine, which turned a resolvable
    // -23.6% result into a wrongly-recorded "unmeasurable".
    println!("
NULL ARM (same code vs itself): median {:.2}%, worst {worst_null:.2}%",
        med(&mut all_null));
    println!("The MEDIAN bounds a median delta; the WORST bounds a single pair.");
    println!("NEITHER bounds the SIGN -- |z| > 2 is a verdict at any floor width.");
    println!("A delta smaller than the null arm is NOT a result -- see codec-measurement 3/15.");
}
