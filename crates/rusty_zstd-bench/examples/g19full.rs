//! GATE 19 @ L1 -- THE FULL DATASET. Every content variable, the whole block-size
//! curve, GATE 5 coverage, and the question never asked: is GATE 5 EARNING where
//! it fires? (G5 disabled via set_g5_fast_arms(2.0, 2.0, 1e9): neither mechanism
//! can bind, so block_max stays at `base` for every block.)
use std::time::Instant;
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
const KB:&[usize]=&[16,32,48,64,96,128];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn g5_on(){ rusty_zstd::set_g5_fast_arms(2.00, 0.70, 2.00); }
fn g5_off(){ rusty_zstd::set_g5_fast_arms(2.00, 2.00, 1.0e9); }
fn t(src:&[u8],n:usize)->f64{
    let mut b=f64::MAX;
    for _ in 0..n{
        let s=Instant::now();
        let z=std::hint::black_box(rusty_zstd::compress(std::hint::black_box(src),1).unwrap());
        let e=s.elapsed().as_secs_f64();
        std::hint::black_box(z.len());
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("== A. CONTENT VARIABLES + GATE 5 COVERAGE ==\n");
    println!("{:<14}{:>8}{:>9}{:>9}{:>9}{:>8}{:>8}{:>9}{:>8}",
        "corpus","ratio","r_prev","drift","pair_gn","rep_yl","tag_yl","blocks","G5 red");
    let mut rows=vec![];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
        g5_on();
        let _=rusty_zstd::take_g5(); let _=rusty_zstd::take_g5_inputs(); let _=rusty_zstd::take_content_signals();
        let z=rusty_zstd::compress(src,1).unwrap().len();
        let (c,r,d)=rusty_zstd::take_g5();
        let (rp,dr)=rusty_zstd::take_g5_inputs();
        let sg=rusty_zstd::take_content_signals();
        let cov=if c>0 {100.0*(r+d) as f64/c as f64} else {0.0};
        let ratio=z as f64/src.len() as f64;
        println!("{id:<14}{ratio:>8.4}{rp:>9.4}{dr:>9.4}{:>9.3}{:>8.3}{:>8.3}{c:>9}{cov:>7.1}%",sg.0,sg.1,sg.2);
        rows.push((id.to_string(),ratio,rp,dr,sg.0,sg.1,cov,z as i64));
    }

    println!("\n== B. BLOCK-SIZE CURVE (size %, vs shipped default) ==\n");
    print!("{:<14}","corpus"); for k in KB{print!("{:>9}",format!("{k}KB"));} println!("{:>8}","best");
    let mut curve=vec![];
    for (id,..) in &rows{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        g5_on();
        unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
        let base=rusty_zstd::compress(src,1).unwrap().len() as f64;
        let mut v=vec![];
        for k in KB{
            unsafe{ std::env::set_var("RZSTD_BLOCK_KB", k.to_string()); }
            v.push(rusty_zstd::compress(src,1).unwrap().len() as f64);
        }
        unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
        let bi=(0..v.len()).min_by_key(|&i|v[i] as i64).unwrap();
        print!("{id:<14}"); for x in &v{ print!("{:>8.3}%",100.0*(x-base)/base); }
        println!("{:>8}",format!("{}KB",KB[bi]));
        curve.push((id.clone(),100.0*(v[bi]-base)/base,KB[bi]));
    }

    println!("\n== C. IS GATE 5 EARNING WHERE IT FIRES? (G5 on vs G5 off) ==\n");
    println!("{:<14}{:>9}{:>11}{:>11}{:>10}","corpus","G5 red","d size","d time","verdict");
    for (id,_,_,_,_,_,cov,_) in &rows{
        if *cov < 1.0 {continue;}
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        g5_off(); let a=rusty_zstd::compress(src,1).unwrap().len() as f64; let ta=t(src,7);
        g5_on();  let b=rusty_zstd::compress(src,1).unwrap().len() as f64; let tb=t(src,7);
        g5_off(); let ta2=t(src,7); let tan=ta.min(ta2);
        let ds=100.0*(b-a)/a; let dt=100.0*(tb-tan)/tan;
        let v=if ds < -0.02 {"earning"} else if ds>0.02 {"COSTS SIZE"} else {"NO SIZE GAIN"};
        println!("{id:<14}{cov:>8.1}%{ds:>+10.3}%{dt:>+10.2}%{v:>10}");
    }
    g5_on();
}
