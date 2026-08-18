//! GATE 6 @ L1: the pair search at ip+1. `pair_forced()` has NO env fallback,
//! so this must drive `set_pair_arm` in-process -- an RZSTD_PAIR env test is a
//! null comparison.
fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    println!("{:<14}{:>13}{:>13}{:>10}", "corpus", "pair OFF", "pair ON", "delta");
    let (mut to, mut tn, mut w, mut l) = (0usize, 0usize, 0, 0);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        rusty_zstd::set_pair_arm(false);
        let a = rusty_zstd::compress(&full, 1).unwrap().len();
        rusty_zstd::set_pair_arm(true);
        let b = rusty_zstd::compress(&full, 1).unwrap().len();
        rusty_zstd::set_pair_arm(false);
        to += a; tn += b;
        let d = 100.0*(b as f64 - a as f64)/a as f64;
        if d < -0.01 { w += 1 } else if d > 0.01 { l += 1 }
        println!("{id:<14}{a:>13}{b:>13}{d:>9.3}%");
    }
    println!("{:<14}{to:>13}{tn:>13}{:>9.3}%", "TOTAL", 100.0*(tn as f64-to as f64)/to as f64);
    println!("\nwins {w}   losses {l}   (of 18)");
    println!("{}", if w>0 && l>0 {"SIGN FLIP -> DISPATCH"} else if w>0 {"all wins -> CONSTANT ON"} else if l>0 {"all losses -> CONSTANT OFF"} else {"no effect"});
}
