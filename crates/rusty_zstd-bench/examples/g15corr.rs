//! GATE 15, one layer deeper: does the words-vs-AVX2 sign track the PREFIX
//! LENGTH distribution? If it does, the CPU gate is missing a content axis.
const IDS:&[&str]=&["x-ray","sao","osdb","smallmsg-8m","mr","dickens","ooffice","webster","reymont","samba","xml","jsonlog-16m","mozilla","nci"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],arm:u8,lvl:i32,r:usize)->f64{
    rusty_zstd::set_eqlen_arm(arm);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:u8,b:u8,lvl:i32)->f64{
    let mut d=vec![];
    for _ in 0..7 { let a1=ms(src,a,lvl,7); let b1=ms(src,b,lvl,7);
        let b2=ms(src,b,lvl,7); let a2=ms(src,a,lvl,7);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[3]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("L{lvl}: negative = the WORD loop is faster than AVX2\n");
    println!("{:<13}{:>10}{:>10}{:>9}{:>10}","corpus","<8B %","words","null","verdict");
    let mut pts=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_eqlen_arm(0);
        let _=rusty_zstd::take_eqlen_stats();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (_c,_w,h)=rusty_zstd::take_eqlen_stats();
        let tot:u64=h.iter().sum();
        if tot==0 {continue;}
        let short=100.0*h[0] as f64/tot as f64;
        let big=&full[..full.len().min(8<<20)];
        let w=paired(big,0,1,lvl); let n=paired(big,0,0,lvl);
        let v=if w < -1.0 {"WORDS win"} else if w>1.0 {"avx2 win"} else {""};
        println!("{id:<13}{short:>9.1}%{w:>9.2}%{n:>8.2}%{v:>10}");
        pts.push((short,w));
    }
    // rank correlation between short-prefix share and the words advantage
    let n=pts.len() as f64;
    let mx=pts.iter().map(|p|p.0).sum::<f64>()/n;
    let my=pts.iter().map(|p|p.1).sum::<f64>()/n;
    let cov:f64=pts.iter().map(|p|(p.0-mx)*(p.1-my)).sum::<f64>();
    let sx:f64=pts.iter().map(|p|(p.0-mx).powi(2)).sum::<f64>().sqrt();
    let sy:f64=pts.iter().map(|p|(p.1-my).powi(2)).sum::<f64>().sqrt();
    println!("\ncorrelation(short-prefix share, words advantage) = {:.3}", cov/(sx*sy));
    println!("(negative = more short prefixes -> words faster, i.e. a real content axis)");
    rusty_zstd::set_eqlen_arm(0);
}
