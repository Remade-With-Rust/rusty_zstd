//! The step dispatch leaves sao/mr/dickens on step 1 where step 2 is FREE.
//! What separates them from samba/mozilla/x-ray, where step 2 costs 9-25%?
const IDS:&[&str]=&["sao","mr","dickens","ooffice","samba","mozilla","x-ray","osdb","webster","nci","xml","reymont"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("L1: step-2 cost against the signals available at L1\n");
    println!("{:<12}{:>10}{:>10}{:>10}{:>10}{:>10}","corpus","size %","mean ml","lit/seq","rep yld","pair gain");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let mut p0=rusty_zstd::compression_params(1,Some(src.len() as u64)).unwrap();
        p0.target_length=0;
        let mut p1=p0; p1.target_length=1;
        let _=rusty_zstd::take_rep_rate(); let _=rusty_zstd::take_pair_stats();
        let a=rusty_zstd::compress_with_params(src,p0,false).unwrap().len() as i64;
        let (_pp,_rb,rh,mb,sq)=rusty_zstd::take_rep_rate();
        let (pb,_ph,pg,_mb2)=rusty_zstd::take_pair_stats();
        let b=rusty_zstd::compress_with_params(src,p1,false).unwrap().len() as i64;
        if sq==0 {continue;}
        rows.push((*id,100.0*(b-a) as f64/a as f64, mb as f64/sq as f64,
            (src.len() as f64-mb as f64)/sq as f64, rh as f64/sq as f64,
            if pb>0 {pg as f64/pb as f64} else {f64::NAN}));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,ml,lp,ry,pg) in &rows{
        let v=if *sd<2.0 {"FREE"} else {"costly"};
        println!("{id:<12}{sd:>+9.3}%{ml:>10.2}{lp:>10.2}{ry:>10.3}{pg:>10.3}  {v}");
    }
}
