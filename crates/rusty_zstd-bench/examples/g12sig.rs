//! GATE 12 @ L3: the end-2 fill sign-flips across content. Instrument FIVE
//! candidate signals against the measured win/loss column before picking an axis.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("{:<14}{:>9}{:>8}{:>9}{:>9}{:>9}{:>9}","corpus","cost%","ml","litfrac","repyld","nlyld","seq/kB");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_fill_n_arm(2);
        let _=rusty_zstd::take_dfast_match_stats(); let _=rusty_zstd::take_pair_stats();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let (mb,sq,bb,_rb,rh)=rusty_zstd::take_dfast_match_stats();
        let (pp,ph,_,_)=rusty_zstd::take_pair_stats();
        rusty_zstd::set_dfast_fill_n_arm(1);
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let cost=100.0*(b-a) as f64/a as f64;
        let ml=if sq>0{mb as f64/sq as f64}else{0.0};
        let lit=if bb>0{1.0-(mb as f64/bb as f64)}else{1.0};
        let rep=if sq>0{rh as f64/sq as f64}else{0.0};
        let nl=if pp>0{ph as f64/pp as f64}else{0.0};
        let sd=if bb>0{1000.0*sq as f64/bb as f64}else{0.0};
        rows.push((*id,cost,ml,lit,rep,nl,sd));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,c,ml,lit,rep,nl,sd) in &rows{
        let m=if *c<=0.001{" FREE"}else{""};
        println!("{id:<14}{c:>+8.3}%{ml:>8.1}{lit:>9.3}{rep:>9.3}{nl:>9.3}{sd:>9.1}{m}");
    }
    rusty_zstd::set_dfast_fill_n_arm(2);
}
