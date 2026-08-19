//! GATE 14 @ L3: the outcome change, per corpus. Baseline = pre-gate (next-long
//! cut 8, second-candidate cut 8). Shipped = dispatched cut + cand2 at 24.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("L{lvl}  baseline = pre-GATE-14 (cut 8, cand2 8)\n");
    println!("{:<14}{:>11}{:>11}{:>9}{:>11}{:>10}","corpus","before","after","bytes","size %","probes %");
    let mut rows=vec![];
    let (mut ta,mut tb,mut pa,mut pb)=(0i64,0i64,0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(8);
        rusty_zstd::set_dfast_good_ml2_arm(8);
        rusty_zstd::set_nl_off_worse_arm(-1.0);
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let xa=rusty_zstd::take_mm().0;
        rusty_zstd::set_dfast_good_ml_arm(0);
        rusty_zstd::set_dfast_good_ml2_arm(0);
        rusty_zstd::set_nl_off_worse_arm(0.60);
        let _=rusty_zstd::take_mm();
        let z=rusty_zstd::compress(src,lvl).unwrap();
        let xb=rusty_zstd::take_mm().0;
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src);
        let b=z.len() as i64;
        ta+=a; tb+=b; pa+=xa; pb+=xb;
        rows.push((*id,a,b,100.0*(b-a) as f64/a as f64,
            if xa>0 {100.0*(xb as f64-xa as f64)/xa as f64} else {0.0}));
    }
    rows.sort_by(|x,y|x.3.partial_cmp(&y.3).unwrap());
    for (id,a,b,sd,pd) in &rows{
        let tag=if *sd < -0.001 {"smaller"} else if *sd>0.001 {"LARGER"} else {"-"};
        println!("{id:<14}{a:>11}{b:>11}{:>9}{sd:>+10.3}%{pd:>+9.2}%  {tag}",b-a);
    }
    println!("\nTOTAL {ta} -> {tb}   {:+} bytes  {:+.4}%   probes {:+.2}%",
        tb-ta, 100.0*(tb-ta) as f64/ta as f64, 100.0*(pb as f64-pa as f64)/pa as f64);
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
    rusty_zstd::set_nl_off_worse_arm(0.60);
}
