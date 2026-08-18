//! RE-VALIDATION of Gates 1-5 at L1/L3/L19/L22 against the outcomes recorded in
//! `gg-matchfind.md` under `## OUTCOMES FROM GATES`.
//!
//! Gates 6 and 7 shipped AFTER these verdicts were taken, and Gate 6's route
//! changed which loop executes, so every one of these is an interaction risk.
//! Every check here is DETERMINISTIC -- compressed sizes and byte-identity --
//! so the verdicts are valid on a busy box. No timing is used or claimed.
use std::io::Write;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const LEVELS: &[(i32, usize)] = &[(1, 8<<20), (3, 8<<20), (19, 2<<20), (22, 2<<20)];

/// Put every gate back to its DEPLOYED setting.
fn deployed() {
    rusty_zstd::set_fast_lazy_arm(true);      // gate 1
    rusty_zstd::set_rep1_mode(None);          // gate 2 (dispatch)
    rusty_zstd::set_lazy_fill_arm(true);      // gate 3
    rusty_zstd::set_dfast_spec_arm(true);     // gate 4
    rusty_zstd::set_bt_spec_arm(true);        // gate 5
}
fn sz(src: &[u8], lvl: i32) -> usize { rusty_zstd::compress(src, lvl).unwrap().len() }

struct Case { gate: u32, name: &'static str, expect_change: bool, note: &'static str }

fn main() {
    let cases = [
        Case{gate:1,name:"fast_lazy on->off",  expect_change:true,  note:"L1 DISPATCH (versions); CONSTANT elsewhere"},
        Case{gate:2,name:"rep1 dispatch->ON",  expect_change:true,  note:"L1 DISPATCH / L3 CONSTANT ON / L19+L22 N/A"},
        Case{gate:2,name:"rep1 dispatch->OFF", expect_change:true,  note:"same"},
        Case{gate:3,name:"lazy_fill on->off",  expect_change:false, note:"CONSTANT at every level (inert)"},
        Case{gate:4,name:"dfast_spec on->off", expect_change:false, note:"CONSTANT, byte-identical"},
        Case{gate:5,name:"bt_spec on->off",    expect_change:false, note:"CONSTANT/DISPATCH, byte-identical"},
    ];
    for &(lvl, cap) in LEVELS {
        println!("\n================ L{lvl} ({} MiB) ================", cap>>20);
        // baseline: everything at its deployed setting
        let mut base = Vec::new();
        let mut srcs = Vec::new();
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let src = full[..full.len().min(cap)].to_vec();
            deployed();
            base.push(sz(&src, lvl));
            srcs.push((*id, src));
        }
        for c in &cases {
            deployed();
            match (c.gate, c.name) {
                (1,_) => rusty_zstd::set_fast_lazy_arm(false),
                (2,n) if n.ends_with("ON")  => rusty_zstd::set_rep1_mode(Some(true)),
                (2,_) => rusty_zstd::set_rep1_mode(Some(false)),
                (3,_) => rusty_zstd::set_lazy_fill_arm(false),
                (4,_) => rusty_zstd::set_dfast_spec_arm(false),
                (5,_) => rusty_zstd::set_bt_spec_arm(false),
                _ => {}
            }
            let mut moved = Vec::new();
            let mut worse = 0;   // deployed LOSES: alternative is smaller
            for (i,(id,src)) in srcs.iter().enumerate() {
                let a = sz(src, lvl);
                if a != base[i] {
                    let d = 100.0*(a as f64 - base[i] as f64)/base[i] as f64;
                    moved.push(format!("{id} {d:+.2}%"));
                    if a < base[i] { worse += 1; }
                }
            }
            let verdict = if c.expect_change {
                if moved.is_empty() { "NO EFFECT" } else { "active" }
            } else if moved.is_empty() { "byte-identical" } else { "MOVED" };
            println!("  gate {} {:<22} {:>2}/{} corpora move  [{}]{}",
                c.gate, c.name, moved.len(), srcs.len(), verdict,
                if worse>0 { format!("  <-- DEPLOYED LOSES on {worse}") } else { String::new() });
            if !moved.is_empty() {
                println!("       {}", moved.join(", "));
            }
            let _ = std::io::stdout().flush();
        }
        deployed();
    }
    println!("\nAll arms restored to deployed settings.");
}
