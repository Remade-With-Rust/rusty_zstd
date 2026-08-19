//! Which half adds the work? Full ledger: positions + next-long probes.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(g1:usize,g2:usize,thr:f32)->(i64,u64,u64){
    let (mut sz,mut pos,mut nl)=(0i64,0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(g1);
        rusty_zstd::set_dfast_good_ml2_arm(g2);
        rusty_zstd::set_nl_off_worse_arm(thr);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_next_long();
        sz+=rusty_zstd::compress(src,3).unwrap().len() as i64;
        pos+=rusty_zstd::take_mm().0; nl+=rusty_zstd::take_next_long().0;
    }
    (sz,pos,nl)
}
fn main(){
    let (bs,bp,bn)=run(8,8,-1.0);
    println!("baseline: size {bs}, positions {bp}, next-long probes {bn}\n");
    println!("{:<26}{:>11}{:>12}{:>12}{:>13}","arm","size %","positions","nl probes","NET ops");
    for (l,g1,g2,t) in [
        ("cand2 24 only",        8usize,24usize,-1.0f32),
        ("next-long dispatched", 0,     8,      0.60),
        ("both (shipped)",       0,     0,      0.60),
        ("next-long forced 24",  24,    8,      2.0),
    ]{
        let (s,p,n)=run(g1,g2,t);
        let net=(p as i64-bp as i64)+(n as i64-bn as i64);
        println!("{l:<26}{:>+10.4}%{:>+12}{:>+12}{:>+13}",
            100.0*(s-bs) as f64/bs as f64, p as i64-bp as i64, n as i64-bn as i64, net);
    }
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
    rusty_zstd::set_nl_off_worse_arm(0.60);
}
