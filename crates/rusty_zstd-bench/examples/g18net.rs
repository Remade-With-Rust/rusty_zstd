//! Per corpus: is route 2 net-CHEAPER in total search ops for any content?
//! The aggregate (+7.80%) can hide corpora where the position saving beats the
//! pair-probe cost. If any exist, a WORK-judged dispatch is real.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("route 1 (shipped) vs route 2, per corpus, COMPLETE work ledger\n");
    println!("{:<13}{:>10}{:>12}{:>12}{:>12}{:>10}","corpus","size %","d positions","d pair","NET ops","verdict");
    let (mut tn,mut ts)=(0i64,0i64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_pair_hi_arm(1.0);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_pair_stats();
        let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0; let qa=rusty_zstd::take_pair_stats().0;
        rusty_zstd::set_pair_hi_arm(0.0);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_pair_stats();
        let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0; let qb=rusty_zstd::take_pair_stats().0;
        let dp=pb as i64-pa as i64; let dq=qb as i64-qa as i64;
        let net=dp+dq;
        if dp==0 && dq==0 && a==b {continue;}
        tn+=net; ts+=b-a;
        let v=if net<0 && b<=a {"FREE WIN"} else if net<0 {"cheaper"} else {"costs"};
        println!("{id:<13}{:>+9.3}%{dp:>12}{dq:>12}{net:>12}{v:>10}",
            100.0*(b-a) as f64/a as f64);
    }
    println!("\nTOTAL net ops {tn:+}, size {ts:+} bytes");
    rusty_zstd::set_pair_hi_arm(1.0);
}
