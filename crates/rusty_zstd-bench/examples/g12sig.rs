//! GATE 12 @ L1 has a SIGN FLIP (4 corpora get SMALLER when end-2 is dropped)
//! that 4.41 never saw. Which of the SIX existing content signals separates it?
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("GATE 12 @ L1 sign-flip vs EVERY existing content signal\n");
    println!("{:<14}{:>10}{:>9}{:>9}{:>9}{:>9}{:>10}{:>10}",
        "corpus","pair_gain","rep_yld","tag_yld","replen","nseq","optrep","d size");
    let mut rows=vec![];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        rusty_zstd::set_dfast_fill_n_arm(2);
        let _=rusty_zstd::take_route_hist(); let _=rusty_zstd::take_content_signals();
        let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let sig=rusty_zstd::take_content_signals(); let (r0,r1,r2,_,_)=rusty_zstd::take_route_hist();
        let _=(r0,r1,r2);

        rusty_zstd::set_dfast_fill_n_arm(1);
        let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let _=rusty_zstd::take_route_hist(); let _=rusty_zstd::take_content_signals();
        if a==b {continue;}
        rows.push((id.to_string(),sig,100.0*(b-a) as f64/a as f64));
    }
    rows.sort_by(|x,y|x.2.partial_cmp(&y.2).unwrap());
    for (id,s,d) in &rows{
        println!("{id:<14}{:>10.4}{:>9.4}{:>9.4}{:>9.4}{:>9.0}{:>10.3}{d:>+9.4}%",
            s.0,s.1,s.2,s.3,s.4,s.5);
    }
    println!("\n(rows sorted by d size: NEGATIVE = dropping end-2 makes it SMALLER)");
    println!("a signal separates iff its values are disjoint across the sign boundary");
    rusty_zstd::set_dfast_fill_n_arm(2);
}
