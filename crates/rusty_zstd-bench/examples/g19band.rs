//! 4.76 band: full corpus, every level. OFF arm disables the band entirely.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    for lv in [1,3,19]{
        println!("\n===== L{lv} =====");
        println!("{:<14}{:>12}{:>12}{:>10}","corpus","OFF","ON","delta");
        let (mut t0,mut t1)=(0i64,0i64);
        for id in IDS{
            let Some(f)=load(id) else{continue};
            let cap=if lv>=19 {2usize<<20} else {8usize<<20};
            let src=&f[..f.len().min(cap)];
            rusty_zstd::set_g5_band_arm(usize::MAX);           // band can never bind
            let a=rusty_zstd::compress(src,lv).unwrap().len() as i64;
            rusty_zstd::set_g5_band_arm(0);                    // shipped default
            let b=rusty_zstd::compress(src,lv).unwrap().len() as i64;
            assert_eq!(rusty_zstd::decompress(&rusty_zstd::compress(src,lv).unwrap()).unwrap(),src,"{id} L{lv}");
            t0+=a; t1+=b;
            if a!=b {
                println!("{id:<14}{a:>12}{b:>12}{:>+9.3}%",100.0*(b-a) as f64/a as f64);
            }
        }
        println!("{:<14}{t0:>12}{t1:>12}{:>+9.4}%","TOTAL",100.0*(t1-t0) as f64/t0 as f64);
    }
    rusty_zstd::set_g5_band_arm(0);
}
