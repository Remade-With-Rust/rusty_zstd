//! GATE 3: does `last_search_per_byte` (the built-in signal) track the PAYOFF?
//! Per corpus: the exchange rate the fill earns, vs the share of its inserts the
//! threshold actually removes. If the threshold cuts hardest where the rate is
//! HIGHEST, the signal is wrong-signed and no threshold on it can ever work.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(7);
    let thr:f32=std::env::args().nth(2).and_then(|s|s.parse().ok()).unwrap_or(0.05);
    println!("{:<14}{:>11}{:>10}{:>10}{:>11}{:>10}","corpus","inserts","B/insert","cut%","size cost","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        // arm A: fill ON, threshold 0 (today)
        rusty_zstd::set_lazy_fill_arm(true);
        rusty_zstd::set_lazy_fill_threshold_arm(0.0);
        let _=rusty_zstd::take_lazy_fill();
        let on=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let (_,_,ins0)=rusty_zstd::take_lazy_fill();
        // arm B: fill OFF entirely -> what the fill is worth
        rusty_zstd::set_lazy_fill_arm(false);
        let off=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        // arm C: fill ON but gated by the threshold
        rusty_zstd::set_lazy_fill_arm(true);
        rusty_zstd::set_lazy_fill_threshold_arm(thr);
        let _=rusty_zstd::take_lazy_fill();
        let gated=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let (_,_,ins1)=rusty_zstd::take_lazy_fill();
        rusty_zstd::set_lazy_fill_threshold_arm(0.0);
        let rate=if ins0>0{(off-on) as f64/ins0 as f64}else{0.0};
        let cut=if ins0>0{100.0*(ins0-ins1) as f64/ins0 as f64}else{0.0};
        let cost=100.0*(gated-on) as f64/on as f64;
        rows.push((*id,ins0,rate,cut,cost));
    }
    rows.sort_by(|a,b|a.2.partial_cmp(&b.2).unwrap());
    for (id,i,r,c,s) in &rows{
        let v=if *c>50.0 && *r<0.01 {"GOOD cut"} else if *c>50.0 {"CUT A WINNER"} else if *r<0.005 {"MISSED waste"} else {""};
        println!("{id:<14}{i:>11}{r:>10.4}{c:>9.1}%{s:>+10.3}%  {v}");
    }
    let ti:i64=rows.iter().map(|r|r.1 as i64).sum();
    println!("\ntotal inserts {ti} @ L{lvl}, threshold {thr}");
}
