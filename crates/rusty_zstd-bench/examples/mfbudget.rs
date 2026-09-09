//! MATCHFIND WORK BUDGET, per input byte, per level. Deterministic, no clock.
//!
//!   cargo run --release --features rusty_zstd/profile -p rusty_zstd-bench --example mfbudget
//!
//! The static census prices a UNIT of work (a scanned position, an examined
//! candidate, a tag-skipped link, a count call, an inserted byte, an emitted
//! match). This example counts how many of each unit run per input byte, so
//! the two multiply into a modelled instruction budget per byte -- and the
//! split says where the encoder's matchfind time goes and which multiplier a
//! reference implementation does not pay.
use std::collections::BTreeMap;

const IDS: &[&str] = &["dickens", "mozilla", "webster", "xml", "samba"];
const CAP: usize = 16 << 20;

#[derive(Default, Clone, Copy)]
struct W {
    bytes: f64,
    out: f64,
    positions: f64, // fast/dfast main-loop positions (MM_TOTAL)
    walks: f64,     // chain/row kernel calls (sum of walk exits)
    exam: f64,      // candidates whose first word was compared
    bytemiss: f64,  // ... of which missed
    tagskip: f64,   // links rejected by the tag alone
    exits: [f64; 8],
    counts: f64, // count_match calls
    count_hist: [f64; 6],
    fused_short: f64,
    fused_long: f64,
    inserts: f64, // fill inserts (lazy/greedy) -- bytes of matches re-inserted
    fills: f64,   // fill sites = matches at lazy/greedy
    dfast_seqs: f64,
    dfast_mb: f64,
    fast_seqs: f64,
    fast_mb: f64,
    rep_probes: f64,
    rep_hits: f64,
    bt_walks: f64,
    bt_iters: f64,
    bt_full: f64,
    bt_probe: f64,
    bt_short: f64,
    bt_nogain: f64,
    routes: [f64; 3],
}

fn reset() {
    let _ = rusty_zstd::take_mm();
    let _ = rusty_zstd::take_walk_exit();
    let _ = rusty_zstd::take_walk_census();
    let _ = rusty_zstd::take_link_tag();
    let _ = rusty_zstd::take_eqlen_stats();
    let _ = rusty_zstd::take_fused();
    let _ = rusty_zstd::take_lazy_fill();
    let _ = rusty_zstd::take_dfast_match_stats();
    let _ = rusty_zstd::take_rep_rate();
    let _ = rusty_zstd::take_bt_iters();
    let _ = rusty_zstd::take_bt_probe_stats();
    let _ = rusted_route();
}

fn rusted_route() -> (u64, u64, u64) {
    let (a, b, c, _, _) = rusty_zstd::take_route_hist();
    (a, b, c)
}

fn main() {
    #[cfg(not(feature = "profile"))]
    {
        println!("needs --features rusty_zstd/profile");
        return;
    }
    #[cfg(feature = "profile")]
    {
        let mut per_level: BTreeMap<i32, W> = BTreeMap::new();
        for lvl in [1i32, 3, 5, 7, 9, 12, 13, 16, 19] {
            let mut acc = W::default();
            for id in IDS {
                let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
                    continue;
                };
                let src = &full[..full.len().min(CAP)];
                reset();
                let out = rusty_zstd::compress(src, lvl).unwrap();
                let (pos, _miss) = rusty_zstd::take_mm();
                let ex = rusty_zstd::take_walk_exit();
                let (exam, bytemiss) = rusty_zstd::take_walk_census();
                let (skips, _false) = rusty_zstd::take_link_tag();
                let (calls, _wide, hist) = rusty_zstd::take_eqlen_stats();
                let (fs, fl) = rusty_zstd::take_fused();
                let (fills, _ne, inserts) = rusty_zstd::take_lazy_fill();
                let (dmb, dseqs, _bb, _drb, _drh) = rusty_zstd::take_dfast_match_stats();
                let (rp, _rb, rh, amb, aseqs) = rusty_zstd::take_rep_rate();
                let (bw, bi, bf) = rusty_zstd::take_bt_iters();
                let (bp, bs, bn) = rusty_zstd::take_bt_probe_stats();
                let (r0, r1, r2) = rusted_route();
                acc.bytes += src.len() as f64;
                acc.out += out.len() as f64;
                acc.positions += pos as f64;
                acc.walks += ex.iter().sum::<u64>() as f64;
                acc.exam += exam as f64;
                acc.bytemiss += bytemiss as f64;
                acc.tagskip += skips as f64;
                for i in 0..8 {
                    acc.exits[i] += ex[i] as f64;
                }
                acc.counts += calls as f64;
                for i in 0..6 {
                    acc.count_hist[i] += hist[i] as f64;
                }
                acc.fused_short += fs as f64;
                acc.fused_long += fl as f64;
                acc.inserts += inserts as f64;
                acc.fills += fills as f64;
                acc.dfast_seqs += dseqs as f64;
                acc.dfast_mb += dmb as f64;
                acc.fast_seqs += aseqs as f64;
                acc.fast_mb += amb as f64;
                acc.rep_probes += rp as f64;
                acc.rep_hits += rh as f64;
                acc.bt_walks += bw as f64;
                acc.bt_iters += bi as f64;
                acc.bt_full += bf as f64;
                acc.bt_probe += bp as f64;
                acc.bt_short += bs as f64;
                acc.bt_nogain += bn as f64;
                acc.routes[0] += r0 as f64;
                acc.routes[1] += r1 as f64;
                acc.routes[2] += r2 as f64;
            }
            per_level.insert(lvl, acc);
        }
        let mib = |w: &W| w.bytes / 1e6;
        println!(
            "board: {} corpora, {} MB\n",
            IDS.len(),
            mib(per_level.values().next().unwrap()) as u64
        );
        println!("=== A. units of work PER INPUT BYTE ===");
        println!(
            "{:>3} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "L",
            "ratio",
            "pos/B",
            "walk/B",
            "exam/B",
            "tagskip",
            "count/B",
            "ins/B",
            "match/B",
            "bytes/m"
        );
        for (lvl, w) in &per_level {
            let matches = if w.fills > 0.0 {
                w.fills
            } else if w.dfast_seqs > 0.0 {
                w.dfast_seqs
            } else {
                w.fast_seqs
            };
            println!(
                "{:>3} {:>7.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.4} {:>8.1}",
                lvl,
                w.bytes / w.out.max(1.0),
                w.positions / w.bytes,
                w.walks / w.bytes,
                w.exam / w.bytes,
                w.tagskip / w.bytes,
                w.counts / w.bytes,
                w.inserts / w.bytes,
                matches / w.bytes,
                if matches > 0.0 {
                    w.bytes / matches
                } else {
                    0.0
                }
            );
        }
        println!("\n=== B. per WALK (chain/row kernel call): candidates examined, tag-skipped, first-word misses, exit reasons ===");
        println!(
            "{:>3} {:>9} {:>8} {:>8} {:>8}   {:<60}",
            "L",
            "walks",
            "exam/wk",
            "skip/wk",
            "miss/ex",
            "exits: nohead ip+mls>len m<low chainend blockend attempts"
        );
        for (lvl, w) in &per_level {
            if w.walks == 0.0 {
                continue;
            }
            let ex: Vec<String> = w.exits[..6]
                .iter()
                .map(|e| format!("{:.1}%", 100.0 * e / w.walks))
                .collect();
            println!(
                "{:>3} {:>9.0} {:>8.2} {:>8.2} {:>7.1}%   {}",
                lvl,
                w.walks,
                w.exam / w.walks,
                w.tagskip / w.walks,
                100.0 * w.bytemiss / w.exam.max(1.0),
                ex.join(" ")
            );
        }
        println!("\n=== C. count_match calls: per byte, and the returned-length histogram [<3, 3-7, 8-31, 32-63, 64-255, 256+] ===");
        for (lvl, w) in &per_level {
            let t: f64 = w.count_hist.iter().sum::<f64>().max(1.0);
            let h: Vec<String> = w
                .count_hist
                .iter()
                .map(|x| format!("{:>5.1}%", 100.0 * x / t))
                .collect();
            println!(
                "{:>3} {:>8.3}/B  {}   fused short {:.1}%",
                lvl,
                w.counts / w.bytes,
                h.join(" "),
                100.0 * w.fused_short / (w.fused_short + w.fused_long).max(1.0)
            );
        }
        println!("\n=== D. binary tree (L13+): walks/B, nodes per walk, walks using ALL attempts, probes too short / no gain ===");
        for (lvl, w) in &per_level {
            if w.bt_walks == 0.0 {
                continue;
            }
            println!(
                "{:>3} {:>8.3}/B  {:>6.1} nodes/walk  {:>5.1}% full  short {:.1}%  nogain {:.1}%",
                lvl,
                w.bt_walks / w.bytes,
                w.bt_iters / w.bt_walks,
                100.0 * w.bt_full / w.bt_walks,
                100.0 * w.bt_short / w.bt_probe.max(1.0),
                100.0 * w.bt_nogain / w.bt_probe.max(1.0)
            );
        }
        println!("\n=== E. rep probes / hits per byte, routes per block ===");
        for (lvl, w) in &per_level {
            println!(
                "{:>3} rep {:>7.3}/B probes, {:>7.4}/B hits ({:.1}% of probes)   routes {:?}",
                lvl,
                w.rep_probes / w.bytes,
                w.rep_hits / w.bytes,
                100.0 * w.rep_hits / w.rep_probes.max(1.0),
                w.routes.map(|r| r as u64)
            );
        }
        // F. the modelled budget: the campaign's measured per-unit costs (emitted-asm path counts)
        println!("\n=== F. MODELLED instructions per input byte = units/byte x per-unit path cost (this crate's measured paths) ===");
        println!(
            "{:>3} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}   {}",
            "L",
            "position",
            "walk",
            "examine",
            "tagskip",
            "count",
            "insert",
            "TOTAL",
            "unit costs used"
        );
        for (lvl, w) in &per_level {
            let b = w.bytes;
            // per-unit costs from the paths tool: position (finder loop, no-match path), walk (kernel entry+exit),
            // examined candidate (first-word reject path), tag-skipped link, count call (entry + 8B/iter),
            // inserted byte (fill loop, packed shape).
            let (c_pos, c_walk, c_exam, c_skip, c_cnt, c_ins) = match lvl {
                1 => (40.0, 0.0, 0.0, 0.0, 75.0, 0.0),
                3 | 4 => (55.0, 0.0, 15.0, 0.0, 75.0, 15.0),
                5 => (60.0, 0.0, 30.0, 26.0, 75.0, 29.0),
                6..=12 => (65.0, 45.0, 25.0, 22.0, 75.0, 29.0),
                13..=15 => (59.0, 105.0, 43.0, 0.0, 75.0, 0.0),
                _ => (80.0, 105.0, 43.0, 0.0, 75.0, 0.0),
            };
            let units_pos = if w.positions > 0.0 {
                w.positions
            } else if w.walks > 0.0 {
                w.walks
            } else {
                b
            };
            let exam = if w.bt_iters > 0.0 { w.bt_iters } else { w.exam };
            let walks = if w.bt_walks > 0.0 {
                w.bt_walks
            } else {
                w.walks
            };
            let p = units_pos * c_pos / b;
            let wk = walks * c_walk / b;
            let e = exam * c_exam / b;
            let s = w.tagskip * c_skip / b;
            let c = w.counts * c_cnt / b;
            let i = w.inserts * c_ins / b;
            println!("{:>3} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}   pos {} walk {} exam {} skip {} cnt {} ins {}",
                     lvl, p, wk, e, s, c, i, p + wk + e + s + c + i, c_pos, c_walk, c_exam, c_skip, c_cnt, c_ins);
        }
    }
}
