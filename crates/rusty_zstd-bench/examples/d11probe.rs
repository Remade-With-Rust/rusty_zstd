//! D11 adjudication: does the DEPTH-1 interleave pay?
//!
//! D9 (same-sequence prefetch, 9.4 ns of distance) read +0.2%, z = -0.91: the
//! out-of-order window covers that distance unaided. D10 (8-deep queue) read
//! +15.8/+18.4%, z = -13/-14: the queue cost more than the latency it hid.
//! D11 is 13.6's original shape -- ONE pending sequence in registers, the next
//! sequence's history load issued a full sequence (~30 ns) early, no queue.
//!
//! Section 21's motive: C's decoder handles 4.5% MORE sequences 1.71x faster
//! (~18 ns/seq vs our ~33), and C's own short decoder interleaves. The stall
//! is ours to hide.
//!
//! Same instrument discipline as d9probe: DecSeqLoop stage isolated, ABBA,
//! null arm printed first, sign test per 14.3 (|z| > 2 is a verdict at any
//! floor width). Round-trip asserted on every decode. --features profile.
use rusty_zstd::ProfStage as S;
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn once(z: &[u8], expect: &[u8], on: bool) -> f64 {
    rusty_zstd::set_pipe1_arm(on);
    rusty_zstd::prof_reset();
    let out = rusty_zstd::decompress(z).unwrap();
    assert!(out == expect, "pipe1={on}: OUTPUT CHANGED -- D11 is not byte-identical");
    rusty_zstd::prof_stage_ns(S::DecSeqLoop) as f64
}
fn med(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n == 0 { return 0.0 }
    if n % 2 == 1 { v[n/2] } else { 0.5*(v[n/2-1]+v[n/2]) }
}
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
    println!("D11 DEPTH-1 INTERLEAVE @ L{lvl} -- DecSeqLoop stage ns/seq, ABBA, {rounds} rounds\n");
    println!("| corpus | seqs | null% | OFF ns/seq | ON ns/seq | delta | wins | z |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    let (mut toff, mut ton, mut tseq) = (0f64, 0f64, 0u64);
    let (mut gw, mut gn) = (0usize, 0usize);
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
        for _ in 0..3 { let _ = once(&z, s, true); }
        let (mut vo, mut vn) = (vec![], vec![]);
        let (mut wins, mut pairs) = (0usize, 0usize);
        for r in 0..rounds {
            let (n1, n2) = abba(&z, s, true, true, r % 2 == 1);
            all_null.push(100.0 * (n1 - n2).abs() / n1.max(n2));
            let (off, on) = abba(&z, s, false, true, r % 2 == 1);
            vo.push(off / n); vn.push(on / n);
            if on < off { wins += 1 }
            pairs += 1;
        }
        let (mo, mn) = (med(&mut vo), med(&mut vn));
        let z_ = (wins as f64 - pairs as f64/2.0) / (0.5*(pairs as f64).sqrt());
        println!("| {id} | {nseq} | {:.1} | {mo:.2} | {mn:.2} | {:+.1}% | {wins}/{pairs} | {z_:+.2} |",
            med(&mut all_null.clone()), 100.0*(mn-mo)/mo);
        toff += mo*n; ton += mn*n; tseq += nseq;
        gw += wins; gn += pairs;
    }
    let (a, b) = (toff/tseq as f64, ton/tseq as f64);
    let gz = (gw as f64 - gn as f64/2.0) / (0.5*(gn as f64).sqrt());
    println!("\n**BOARD: OFF {a:.2} ns/seq -> ON {b:.2} ns/seq = {:+.1}%  |  {gw}/{gn} pairs, z = {gz:+.2}**",
        100.0*(b-a)/a);
    println!("\nNULL ARM: median {:.2}% (the sign test decides regardless -- 14.3)", med(&mut all_null));
}
