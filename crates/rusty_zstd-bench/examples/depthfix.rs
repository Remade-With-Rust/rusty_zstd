//! The depth gate's four per-call `std::env::var` lookups, cached. Estimated at
//! 60% of L19 encode and 50% of L22 -- measure it.
const IDS:&[&str]=&["dickens","samba","nci","xml","mozilla","webster","reymont","mr","ooffice","osdb","sao","x-ray"];
fn ms(src:&[u8],cached:bool,lvl:i32,r:usize)->f64{
    rusty_zstd::set_bt_depth_cached_arm(cached);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    println!("L{lvl}: negative = the cached build is FASTER\n");
    println!("{:<12}{:>11}{:>11}{:>10}","corpus","env ms","cached ms","delta");
    let (mut ta,mut tb)=(0.0,0.0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        // ABBA
        let a1=ms(src,false,lvl,2); let b1=ms(src,true,lvl,2);
        let b2=ms(src,true,lvl,2);  let a2=ms(src,false,lvl,2);
        let a=a1.min(a2); let b=b1.min(b2);
        ta+=a; tb+=b;
        println!("{id:<12}{a:>11.1}{b:>11.1}{:>9.1}%",100.0*(b-a)/a);
    }
    println!("\nTOTAL {ta:.0} ms -> {tb:.0} ms   {:+.1}%", 100.0*(tb-ta)/ta);
    rusty_zstd::set_bt_depth_cached_arm(true);
}
