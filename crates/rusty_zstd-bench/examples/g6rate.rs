//! What EXCHANGE RATE does the pair search get, per corpus? bytes covered per
//! probe spent. If the losers separate from the winners here, this is the axis.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    rusty_zstd::set_pair_on_arm(true);
    rusty_zstd::set_pair_gain_arm(0.0); // never gate: measure the rate everywhere
    println!("{:<14}{:>12}{:>12}{:>10}{:>12}", "corpus", "probes", "hit bytes", "B/probe", "size vs off");
    println!("{}", "-".repeat(60));
    let mut rows = vec![];
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        rusty_zstd::set_pair_on_arm(false);
        let off = rusty_zstd::compress(src,1).unwrap().len() as f64;
        rusty_zstd::set_pair_on_arm(true);
        let _ = rusty_zstd::take_pair_stats();
        let on = rusty_zstd::compress(src,1).unwrap().len() as f64;
        let (probes, _h, bytes, _x) = rusty_zstd::take_pair_stats();
        let rate = bytes as f64 / (probes.max(1)) as f64;
        let d = 100.0*(on-off)/off;
        println!("{id:<14}{probes:>12}{bytes:>12}{rate:>10.3}{d:>11.2}%");
        rows.push((id, rate, d));
    }
    rows.sort_by(|a,b| a.1.partial_cmp(&b.1).unwrap());
    println!("\nsorted by rate (does it separate winners from losers?)");
    for (id, r, d) in &rows { println!("  {r:>7.3} B/probe  {d:>7.2}%  {id}"); }
}
