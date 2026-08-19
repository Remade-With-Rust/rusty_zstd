//! GATE 14 @ L3 DISPATCH: does gating the raise on the offset-trade signal keep
//! the win on the 11 winners while protecting mr and osdb?
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("L{lvl}: baseline = next-long cut pinned at 8 (pre-GATE-14)\n");
    println!("{:<14}{:>12}{:>12}{:>11}{:>11}","corpus","forced 24","dispatched","forced %","dispatch %");
    let (mut t8,mut t24,mut td,mut p8,mut pd)=(0i64,0i64,0i64,0u64,0u64);
    let (mut w,mut wid)=(0f64,"");
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        // pinned 8 = the old behaviour
        rusty_zstd::set_nl_off_worse_arm(-1.0);   // never <= threshold -> always 8 after warmup
        rusty_zstd::set_dfast_good_ml_arm(8);
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        p8+=rusty_zstd::take_mm().0;
        // forced 24
        rusty_zstd::set_dfast_good_ml_arm(24);
        rusty_zstd::set_nl_off_worse_arm(2.0);    // always <= threshold -> always raised
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        // dispatched
        rusty_zstd::set_nl_off_worse_arm(0.70);
        let _=rusty_zstd::take_mm();
        let z=rusty_zstd::compress(src,lvl).unwrap();
        pd+=rusty_zstd::take_mm().0;
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip {id}");
        let d=z.len() as i64;
        let fd=100.0*(b-a) as f64/a as f64;
        let dd=100.0*(d-a) as f64/a as f64;
        if dd>w {w=dd; wid=id;}
        t8+=a; t24+=b; td+=d;
        if a!=b || a!=d {
            println!("{id:<14}{b:>12}{d:>12}{fd:>+10.3}%{dd:>+10.3}%");
        }
    }
    println!("\nTOTAL   forced24 {:+.4}%   dispatched {:+.4}%   probes {:+.2}%   worst {} {:+.3}%",
        100.0*(t24-t8) as f64/t8 as f64, 100.0*(td-t8) as f64/t8 as f64,
        100.0*(pd as f64-p8 as f64)/p8 as f64, wid, w);
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_nl_off_worse_arm(0.70);
}
