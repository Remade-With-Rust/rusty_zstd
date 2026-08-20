//! Price the hash-width divergence WITHOUT shipping it. We hash 4 bytes at
//! every min_match; C's ZSTD_hashPtr hashes mls bytes (6 at L2, 7 at L1). Every
//! candidate whose 4 bytes match but whose true length dies below mls is work a
//! wider hash would never have surfaced: one random src[m] load + compare +
//! count_match, wasted. Deterministic; the case for the (output-changing) mls
//! hash rests on this number.
const IDS: &[&str] = &["dickens","reymont","webster","samba","sao","mr","jsonlog-16m","nci","mozilla","osdb","ooffice","x-ray"];
fn main() {
    for lvl in [1i32, 2] {
        println!("L{lvl} (min_match {})", if lvl==1 {7} else {6});
        println!("  {:<12} {:>12} {:>12} {:>10}", "corpus", "4B passes", "accepted", "WASTED%");
        let (mut tc, mut ta) = (0u64, 0u64);
        for id in IDS {
            let Ok(f)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s=&f[..f.len().min(8<<20)];
            let _=rusty_zstd::take_ff_waste();
            let _=rusty_zstd::compress(s,lvl).unwrap();
            let (c4,acc)=rusty_zstd::take_ff_waste();
            tc+=c4; ta+=acc;
            println!("  {:<12} {:>12} {:>12} {:>9.1}%", id, c4, acc,
                (c4-acc) as f64/c4.max(1) as f64*100.0);
        }
        println!("  TOTAL: {tc} four-byte passes, {ta} accepted -> {} wasted ({:.1}%)\n",
            tc-ta, (tc-ta) as f64/tc.max(1) as f64*100.0);
    }
}
