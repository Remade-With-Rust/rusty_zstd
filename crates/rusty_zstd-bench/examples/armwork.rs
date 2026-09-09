//! ARM WORK CENSUS -- the proof `allgates` explicitly leaves open.
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example armwork
//!
//! `allgates` reports 13 arms SZ-DEAD and then says the honest thing: "SZ-DEAD
//! on a byte-identical SPEED capability = the identity proof it owes; the speed
//! question is still open and belongs on the clock." On this box the clock has
//! a +-1.5% null band (see `eqlever.rs`), so for a capability worth ~1% the
//! clock cannot answer it -- ever.
//!
//! But a speed capability that is real must move some WORK counter: fewer
//! probes, fewer fills, fewer copied bytes, fewer allocations. If toggling an
//! arm moves neither the output bytes NOR any work counter, the arm is dead in
//! both currencies and no clock is needed to say so.
//!
//! A row that moves nothing is a REMOVABLE BRANCH. A row that moves work but
//! not bytes is a live byte-identical capability -- exactly what it claims.
use rusty_zstd as rz;

const IDS: &[&str] = &["dickens", "webster", "mozilla", "samba", "nci", "x-ray",
                       "osdb", "sao", "jsonlog-16m", "smallmsg-8m"];

#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct Work { bytes: u64, probes: u64, fills: u64, hits: u64, seqs: u64,
              allocs: u64, pos: u64, copy: u64, lit: u64, mb: u64 }

fn run(lvl: i32, srcs: &[Vec<u8>]) -> Work {
    let mut w = Work::default();
    let _ = rz::take_mm();
    let _ = rz::copies::take();
    for s in srcs {
        rz::prof_reset();
        let out = rz::compress(s, lvl).unwrap();
        w.bytes += out.len() as u64;
        let c = rz::prof_encode_counts();
        w.probes += c.hash_probes; w.fills += c.hash_fills; w.hits += c.probe_hits;
        w.seqs += c.seqs; w.allocs += c.scratch_allocs;
        w.lit += c.lit_bytes; w.mb += c.match_bytes;
    }
    w.pos = rz::take_mm().0;
    w.copy = rz::copies::take().iter().map(|x| x.1).sum();
    w
}

fn main() {
    let cap: usize = 4 << 20;
    let srcs: Vec<Vec<u8>> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
            .ok().map(|f| f[..f.len().min(cap)].to_vec())
    }).collect();
    println!("board: {} corpora, {} MiB\n", srcs.len(),
             srcs.iter().map(|s| s.len()).sum::<usize>() >> 20);

    type Setter = fn(bool);
    let arms: &[(&str, Setter, bool, i32)] = &[
        ("pipe",            rz::set_pipe_arm as Setter,          true, 1),
        ("fast_spec",       rz::set_fast_spec_arm,               true, 1),
        ("litpush",         rz::set_litpush_arm,                 true, 1),
        ("litpush_hoist",   rz::set_litpush_hoist_arm,           true, 1),
        ("payload_reserve", rz::set_payload_arm,                 true, 1),
        ("huff_fast",       rz::set_huff_fast_arm,               true, 1),
        ("finder_scratch",  rz::set_finder_scratch_arm,          true, 1),
        ("fast_lazy",       rz::set_fast_lazy_arm,               true, 1),
        ("dfast_pipe",      rz::set_dfast_pipe_arm,              true, 3),
        ("dfast_spec",      rz::set_dfast_spec_arm,              true, 3),
        ("dfast_tag",       rz::set_dfast_tag_arm,               true, 3),
        ("lazy_fill",       rz::set_lazy_fill_arm,               true, 9),
    ];
    // CONTROL FIRST. A zero delta is only evidence once the counter is proven
    // REACHED -- the lesson this session already paid for twice. Print the
    // ABSOLUTE baseline of every counter before any delta is believed.
    let base = run(1, &srcs);
    println!("CONTROL (L1 defaults, absolute): bytes {} probes {} fills {} pos {} copyB {} allocs {} seqs {}",
             base.bytes, base.probes, base.fills, base.pos, base.copy, base.allocs, base.seqs);
    let b3 = run(3, &srcs);
    println!("CONTROL (L3 defaults, absolute): bytes {} probes {} fills {} pos {} copyB {} allocs {} seqs {}
",
             b3.bytes, b3.probes, b3.fills, b3.pos, b3.copy, b3.allocs, b3.seqs);
    println!("{:<17}{:>3} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}   {}",
             "arm", "L", "d bytes", "d probes", "d fills", "d pos", "d copyB", "d allocs", "verdict");
    println!("{}", "-".repeat(108));
    for (name, set, deflt, lvl) in arms {
        set(*deflt);
        let a = run(*lvl, &srcs);
        set(!*deflt);
        let b = run(*lvl, &srcs);
        set(*deflt); // restore
        let d = |x: u64, y: u64| y as i64 - x as i64;
        let (db, dp, df, dc, da) = (d(a.bytes,b.bytes), d(a.probes,b.probes),
                                    d(a.fills,b.fills), d(a.copy,b.copy), d(a.allocs,b.allocs));
        let moved_work = dp != 0 || df != 0 || dc != 0 || da != 0
                         || a.pos != b.pos || a.seqs != b.seqs;
        let verdict = if db != 0 { "LIVE (changes bytes)" }
                      else if moved_work { "live: byte-identical, moves work" }
                      else { "DEAD IN BOTH -- removable branch" };
        println!("{:<17}{:>3} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}   {}",
                 name, lvl, db, dp, df, d(a.pos, b.pos), dc, da, verdict);
    }
}
