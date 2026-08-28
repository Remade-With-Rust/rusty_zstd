//! D9 COVERAGE -- the deterministic number that SIZES the prefetch brick.
//!
//! The d9probe clock could not adjudicate D9: +0.2% against a null arm of
//! 27.7% on a box carrying faucet.exe at 86,147 CPU-s. So do what
//! codec-measurement 15 says and make the COUNTER primary.
//!
//! D9 only prefetches sequences whose offset is a LITERAL offset
//! (`offset_value > 3`), because a REP code needs `state.reps`, which is
//! resolved after the prefetch point. So D9's ceiling is not `copy_match`'s
//! 57.6% of the loop -- it is that share TIMES the literal-offset rate.
//! This prints the rate. It is the arithmetic that should have run BEFORE the
//! brick was written (codec-measurement 11: prune on arithmetic first).
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    println!("D9 PREFETCH COVERAGE @ L{lvl} -- deterministic, no clock\n");
    println!("{:<14}{:>12}{:>12}{:>12}{:>11}{:>13}",
        "corpus", "issued", "skip rep", "skip oob", "covered", "D9 ceiling");
    let (mut ti, mut tr, mut to) = (0u64, 0u64, 0u64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let s = &f[..f.len().min(8 << 20)];
        let z = rusty_zstd::compress(s, lvl).unwrap();
        let _ = rusty_zstd::take_pf_census();
        let out = rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out, s);
        let (i, r, o) = rusty_zstd::take_pf_census();
        let tot = (i + r + o) as f64;
        if tot < 1000.0 { continue }
        let cov = 100.0 * i as f64 / tot;
        // copy_match is 57.6% of DecSeqLoop (dsloop ladder). D9 can hide at most
        // the ~9.4 ns of work that precedes it, out of copy_match's 23.29 ns.
        let ceil = 57.6 * (cov / 100.0) * (9.4 / 23.29);
        println!("{id:<14}{i:>12}{r:>12}{o:>12}{cov:>10.1}%{ceil:>12.1}%");
        ti += i; tr += r; to += o;
    }
    let tot = (ti + tr + to) as f64;
    let cov = 100.0 * ti as f64 / tot;
    println!("\nBOARD: {ti} issued / {tr} rep-skipped / {to} oob-skipped");
    println!("COVERAGE {cov:.1}% of sequences -- D9 cannot touch the other {:.1}%.", 100.0 - cov);
    println!("CEILING  {:.1}% of DecSeqLoop, if every issued prefetch fully hid its miss.",
        57.6 * (cov / 100.0) * (9.4 / 23.29));
    println!("\n(57.6% = copy_match's share of the loop; 9.4/23.29 = the work available");
    println!(" to overlap, over copy_match's measured cost. Both from the dsloop ladder.)");
}
