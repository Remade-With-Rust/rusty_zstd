//! GATE 2 @ L3 is CONSTANT ON (`rep_yield_min_for(DFast) == 0.0`) and it LOSES
//! on 7 of 18. Find the axis. Candidates, all measured on the ON arm:
//!   rep_yield      rep hits / sequences        (today's L1 variable)
//!   rep_len_ratio  mean rep len / mean match len   (the L1 SECOND variable)
//!   rep_share      rep bytes / all match bytes
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let mut rows=vec![];
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_rep1_mode(None);            // deployed: constant ON at L3
        let _=rusty_zstd::take_dfast_match_stats();
        let on=rusty_zstd::compress(src,3).unwrap().len() as f64;
        let (mb,sq,_bb,rb,rh)=rusty_zstd::take_dfast_match_stats();
        rusty_zstd::set_rep1_mode(Some(false));     // forced OFF
        let off=rusty_zstd::compress(src,3).unwrap().len() as f64;
        rusty_zstd::set_rep1_mode(None);
        let mean_ml = mb as f64/sq.max(1) as f64;
        let rep_len = rb as f64/rh.max(1) as f64;
        let ratio = if mean_ml>0.0 {rep_len/mean_ml} else {0.0};
        let yield_ = rh as f64/sq.max(1) as f64;
        let share = rb as f64/mb.max(1) as f64;
        // POSITIVE delta = turning rep OFF makes it SMALLER = the constant LOSES
        rows.push((*id, 100.0*(on-off)/off, yield_, ratio, share));
    }
    rows.sort_by(|a,b| a.3.partial_cmp(&b.3).unwrap());
    println!("{:<14}{:>12}{:>10}{:>10}{:>10}", "corpus","ON vs OFF","rep_yld","len_ratio","rep_share");
    println!("  positive = the CONSTANT-ON loses (OFF would be smaller)\n");
    for (id,d,y,r,sh) in &rows { println!("{id:<14}{d:>11.3}%{y:>10.3}{r:>10.3}{sh:>10.3}"); }
}
