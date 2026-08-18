//! Gate 6 candidate variables: does pair_yield or pair_gain separate the
//! winners from jsonlog (the residual loser)?
fn main() {
    let ids = ["nci","mozilla","reymont","samba","xml","webster","dickens","ooffice","osdb","mr",
               "sao","smallmsg-8m","text-32m","x-ray","jsonlog-16m","versions-16m"];
    // deltas measured earlier, pair ON vs OFF at L1
    let delta = [(-13.243),(-9.665),(-9.029),(-8.013),(-7.813),(-7.747),(-7.440),(-7.427),(-5.132),
                 (-3.024),(-1.857),(-1.373),(-1.192),(-0.022),(0.178),(10.553)];
    println!("{:<14}{:>9}{:>12}{:>12}{:>12}   outcome", "corpus", "delta%", "pair_yield", "pair_bytes", "probes/blk");
    for (i, id) in ids.iter().enumerate() {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        rusty_zstd::set_pair_on_arm(true);
        let _ = rusty_zstd::take_pair_stats();
        let _ = rusty_zstd::compress(&full, 1).unwrap();
        let (pp, ph, pb, _) = rusty_zstd::take_pair_stats();
        rusty_zstd::set_pair_on_arm(true);
        let blocks = (full.len() / 131072).max(1) as u64;
        let yld = if pp > 0 { ph as f64 / pp as f64 } else { 0.0 };
        let gain = pb as f64 / full.len() as f64;
        let tag = if delta[i] < -0.5 { "WINS" } else if delta[i] > 0.05 { "LOSES" } else { "flat" };
        println!("{id:<14}{:>9.2}{yld:>12.4}{gain:>12.4}{:>12}   {tag}", delta[i], pp/blocks);
    }
}
