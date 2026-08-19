//! The specialised body must be byte-identical to the runtime body it replaces,
//! at the sizes that previously fell through: 64K, 512K, 1M. Then time it there.
const IDS:&[&str]=&["dickens","samba","nci","xml","mozilla","webster","reymont","mr","ooffice","osdb","sao","x-ray"];
fn ms(src:&[u8],spec:bool,lvl:i32,r:usize)->f64{
    rusty_zstd::set_bt_spec_arm(spec);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:bool,b:bool,lvl:i32,r:usize)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,lvl,r); let b1=ms(src,b,lvl,r);
        let b2=ms(src,b,lvl,r); let a2=ms(src,a,lvl,r);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    let n:usize=std::env::args().nth(2).and_then(|s|s.parse().ok()).unwrap_or(512<<10);
    // 1. byte-identity, every corpus
    let mut bad=0;
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(n)];
        rusty_zstd::set_bt_spec_arm(false);
        let a=rusty_zstd::compress(src,lvl).unwrap();
        rusty_zstd::set_bt_spec_arm(true);
        let b=rusty_zstd::compress(src,lvl).unwrap();
        if a!=b { println!("MISMATCH {id}: runtime {} vs specialised {}",a.len(),b.len()); bad+=1; }
        assert_eq!(rusty_zstd::decompress(&b).unwrap(), src, "roundtrip {id}");
    }
    println!("byte-identity at L{lvl}, {} KiB: {} mismatches (roundtrip OK)", n>>10, bad);
    // 2. timing
    println!("\nnegative = the specialised body is FASTER");
    println!("{:<12}{:>9}{:>11}","corpus","null","spec");
    let (mut tn,mut ts,mut k)=(0.0,0.0,0.0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(n)];
        let nl=paired(src,false,false,lvl,5);
        let sp=paired(src,false,true,lvl,5);
        println!("{id:<12}{nl:>8.2}%{sp:>10.2}%");
        tn+=nl.abs(); ts+=sp; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean spec {:+.2}%", tn/k, ts/k);
    rusty_zstd::set_bt_spec_arm(true);
}
