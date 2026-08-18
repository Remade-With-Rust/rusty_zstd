//! Does mean MATCH LENGTH (and match coverage) predict where DFast step-2 is
//! free? Skipping odd positions loses a byte off a LONG match and loses a SHORT
//! match entirely -- and loses nothing where there are no matches at all.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let mut rows=vec![];
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_dfast_step_arm(1);
        let _=rusty_zstd::take_dfast_match_stats();
        let s1=rusty_zstd::compress(src,3).unwrap().len() as f64;
        let (mb,sq,bb)=rusty_zstd::take_dfast_match_stats();
        rusty_zstd::set_dfast_step_arm(2);
        let s2=rusty_zstd::compress(src,3).unwrap().len() as f64;
        let mean_ml = mb as f64/sq.max(1) as f64;
        let cover = 100.0*mb as f64/bb.max(1) as f64;
        rows.push((*id, mean_ml, cover, 100.0*(s2-s1)/s1));
    }
    rows.sort_by(|a,b| a.1.partial_cmp(&b.1).unwrap());
    println!("{:<14}{:>10}{:>10}{:>12}", "corpus","mean ml","cover%","step2 size");
    for (id,ml,cv,d) in &rows { println!("{id:<14}{ml:>10.2}{cv:>9.1}%{d:>11.2}%"); }
    rusty_zstd::set_dfast_step_arm(1);
}
