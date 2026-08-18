//! L19 on its OWN terms: how much of the binary-tree walk is wasted?
fn main() {
    println!("{:<10}{:>13}{:>13}{:>9}{:>13}{:>9}", "corpus","bt probes","too short","%","no gain","%");
    let (mut tp, mut ts, mut tn) = (0u64,0u64,0u64);
    for id in ["xml","osdb","nci","webster","mozilla","dickens","samba","reymont"] {
        let full = std::fs::read(format!("corpora/data/silesia/{id}")).unwrap();
        let src = &full[..full.len().min(1024*1024)];
        let _ = rusty_zstd::take_bt_probe_stats();
        let _ = rusty_zstd::compress(src, 19).unwrap();
        let (p, sh, ng) = rusty_zstd::take_bt_probe_stats();
        tp+=p; ts+=sh; tn+=ng;
        println!("{id:<10}{p:>13}{sh:>13}{:>8.1}%{ng:>13}{:>8.1}%",
                 100.0*sh as f64/p.max(1) as f64, 100.0*ng as f64/p.max(1) as f64);
    }
    println!("{:<10}{tp:>13}{ts:>13}{:>8.1}%{tn:>13}{:>8.1}%", "TOTAL",
             100.0*ts as f64/tp.max(1) as f64, 100.0*tn as f64/tp.max(1) as f64);
}
