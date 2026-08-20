//! The win is in the ROUTE, not the step. Isolate pair_route = 2 (via GATE 6's
//! hi threshold) with the step dispatch left alone.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("L1: forcing pair_route 2 (hi threshold -> 0), step dispatch untouched\n");
    println!("{:<14}{:>11}{:>10}{:>13}{:>10}","corpus","size delta","size %","positions","pos %");
    let (mut ta,mut tb,mut pa,mut pb)=(0i64,0i64,0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_step_probe_arm(false);
        rusty_zstd::set_pair_hi_arm(1.0);
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let xa=rusty_zstd::take_mm().0;
        rusty_zstd::set_pair_hi_arm(0.0);
        let _=rusty_zstd::take_mm();
        let z=rusty_zstd::compress(src,1).unwrap();
        let xb=rusty_zstd::take_mm().0;
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip {id}");
        let b=z.len() as i64;
        ta+=a; tb+=b; pa+=xa; pb+=xb;
        if a!=b || xa!=xb {
            println!("{id:<14}{:>11}{:>9.3}%{:>13}{:>9.2}%",b-a,
                100.0*(b-a) as f64/a as f64, xa as i64-xb as i64,
                100.0*(xb as f64-xa as f64)/xa as f64);
        }
    }
    println!("\nTOTAL size {:+} ({:+.4}%)   positions {:+} ({:+.2}%)",
        tb-ta,100.0*(tb-ta) as f64/ta as f64,
        pb as i64-pa as i64,100.0*(pb as f64-pa as f64)/pa as f64);
    rusty_zstd::set_pair_hi_arm(1.0);
}
