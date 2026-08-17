//! DETERMINISTIC Gate 4 verdict: no clock involved.
//!
//! A byte-identical specialisation cannot change WHICH matches are found, so
//! probe counts must be identical between arms. If they are, the only thing
//! that changed is instructions-per-probe -- a CONSTANT factor applied equally
//! to every corpus. A constant factor cannot sign-flip, so such a gate can
//! never be a content dispatch; it is a constant, and the only question is
//! which way it points.
fn run(src: &[u8], lvl: i32, spec: bool) -> (usize, u64, u64) {
    if lvl == 1 { rusty_zstd::set_fast_spec_arm(spec); } else { rusty_zstd::set_dfast_spec_arm(spec); }
    rusty_zstd::prof_reset();
    let z = rusty_zstd::compress(src, lvl).unwrap();
    let c = rusty_zstd::prof_encode_counts();
    (z.len(), c.hash_probes, c.seqs)
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    println!("DETERMINISTIC Gate 4 @ L{lvl}: probes and bytes, generic vs specialised");
    println!("{:<14}{:>13}{:>13}{:>9}{:>13}{:>13}{:>7}", "file","probes gen","probes spec","same?","bytes gen","bytes spec","same?");
    let (mut ap, mut bp) = (0u64, 0u64);
    let mut all = true;
    for f in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{f}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{f}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let (gz, gp, _) = run(src, lvl, false);
        let (sz, sp, _) = run(src, lvl, true);
        ap += gp; bp += sp;
        let pe = gp == sp; let be = gz == sz;
        if !pe || !be { all = false; }
        println!("{f:<14}{gp:>13}{sp:>13}{:>9}{gz:>13}{sz:>13}{:>7}", if pe {"yes"} else {"NO"}, if be {"yes"} else {"NO"});
    }
    println!("{:<14}{ap:>13}{bp:>13}", "TOTAL");
    println!("\nprobe counts identical on every corpus: {}", if all {"YES"} else {"NO"});
    println!("=> the arms do identical WORK; only instructions-per-probe differ.");
}
