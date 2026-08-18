//! GATE 2 @ L3 as a SPEED question. `try_rep1` runs at EVERY position in DFast
//! and is constant-ON. On low-yield content it is a probe per position that
//! finds nothing. Null arm first.
const IDS: &[&str] = &["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn ms(src:&[u8],on:Option<bool>,n:usize)->f64{ rusty_zstd::set_rep1_mode(on); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn paired(src:&[u8],a:Option<bool>,b:Option<bool>)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("{:<14}{:>9}{:>10}{:>11}{:>10}", "corpus","rep_yld","null","size OFF","time OFF");
    println!("  negative time = turning the repcode search OFF is FASTER\n");
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_rep1_mode(None);
        let _=rusty_zstd::take_dfast_match_stats();
        let on=rusty_zstd::compress(src,3).unwrap().len() as f64;
        let (_mb,sq,_bb,_rb,rh)=rusty_zstd::take_dfast_match_stats();
        rusty_zstd::set_rep1_mode(Some(false));
        let off=rusty_zstd::compress(src,3).unwrap().len() as f64;
        let null=paired(src,None,None);
        let t=paired(src,None,Some(false));
        println!("{id:<14}{:>9.3}{null:>9.2}%{:>10.3}%{t:>9.2}%",
            rh as f64/sq.max(1) as f64, 100.0*(off-on)/on);
        rusty_zstd::set_rep1_mode(None);
    }
}
