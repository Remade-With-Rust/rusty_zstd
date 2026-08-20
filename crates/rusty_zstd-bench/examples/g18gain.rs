//! VALIDATE the candidate: does pair_gain < ~0.71 predict "route 2 is net
//! CHEAPER in total search ops"? Full corpus, not the six that suggested it.
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("Does pair_gain predict the WORK sign? (route 1 -> route 2)\n");
    println!("{:<14}{:>10}{:>12}{:>12}{:>12}{:>9}{:>10}","corpus","pair_gain","d positions","d pair","NET ops","size %","predicted");
    let mut rows=vec![];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        // arm A: shipped
        rusty_zstd::set_pair_hi_arm(1.0);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_pair_stats(); let _=rusty_zstd::take_route_hist();
        let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0; let qa=rusty_zstd::take_pair_stats().0;
        let g=rusty_zstd::take_route_hist().3;
        // arm B: force route 2 everywhere
        rusty_zstd::set_pair_hi_arm(0.0);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_pair_stats();
        let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0; let qb=rusty_zstd::take_pair_stats().0;
        let net=(pb as i64-pa as i64)+(qb as i64-qa as i64);
        if net==0 && a==b {continue;}
        rows.push((id.to_string(),g,net,100.0*(b-a) as f64/a as f64,
            pb as i64-pa as i64, qb as i64-qa as i64));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    let (mut ok,mut tot)=(0,0);
    for (id,g,net,sz,dp,dq) in &rows{
        let pred=*g<0.71; let actual=*net<0;
        if pred==actual {ok+=1;} tot+=1;
        println!("{id:<14}{g:>10.4}{dp:>12}{dq:>12}{net:>12}{sz:>+8.3}%{:>10}",
            if pred==actual {"OK"} else {"MISS"});
    }
    println!("\nthreshold pair_gain < 0.71 predicts the work sign on {ok}/{tot} corpora");
    rusty_zstd::set_pair_hi_arm(1.0);
}
