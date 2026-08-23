//! SIMD-3 adjudication: AVX2+BMI2 entropy twins vs the bmi2-only arm.
//!
//! ABBA interleaved in process. The arms MUST produce identical bytes -- same
//! `#[inline(always)]` body, different `target_feature` -- and that is asserted
//! on every single compress, so a wrong arm cannot look like a fast arm.
use rusty_zstd::ProfStage as S;
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
/// (huff, fseseq, seqcode, encode_total) ns for one compress, output checked.
fn run(src:&[u8], lvl:i32, on:bool, expect:&[u8])->(f64,f64,f64,f64){
    rusty_zstd::set_enc_avx2_arm(on);
    rusty_zstd::prof_reset();
    let z=rusty_zstd::compress(src,lvl).unwrap();
    assert!(z==expect, "arm avx2={on}: OUTPUT CHANGED -- twins are not byte-identical");
    (rusty_zstd::prof_stage_ns(S::EncodeHuff) as f64,
     rusty_zstd::prof_stage_ns(S::EncodeFseSeq) as f64,
     rusty_zstd::prof_stage_ns(S::EncodeSeqCode) as f64,
     rusty_zstd::prof_stage_ns(S::EncodeTotal) as f64)
}
fn med(v:&mut Vec<f64>)->f64{
    v.sort_by(|a,b|a.partial_cmp(b).unwrap());
    let n=v.len(); if n==0 {return 0.0}
    if n%2==1 {v[n/2]} else {0.5*(v[n/2-1]+v[n/2])}
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let rounds:usize=std::env::args().nth(2).and_then(|s|s.parse().ok()).unwrap_or(7);
    let cap=8usize<<20;
    println!("SIMD-3: AVX2 entropy twins vs bmi2-only, ABBA x{rounds} @ L{lvl}");
    println!("(ms totals; negative = AVX2 faster; every compress asserted byte-identical)\n");
    println!("| corpus | Huff OFF | Huff ON | d% | FseSeq OFF | FseSeq ON | d% | SeqCode OFF | SeqCode ON | d% | encode d% |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    let mut t=[0f64;8];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let s=&f[..f.len().min(cap)];
        rusty_zstd::set_enc_avx2_arm(true);
        let expect=rusty_zstd::compress(s,lvl).unwrap();
        for _ in 0..2 { let _=run(s,lvl,true,&expect); }
        let mut c:[Vec<f64>;8]=Default::default();
        for _ in 0..rounds{
            let a1=run(s,lvl,false,&expect); let b1=run(s,lvl,true,&expect);
            let b2=run(s,lvl,true,&expect);  let a2=run(s,lvl,false,&expect);
            c[0].push(a1.0.min(a2.0)); c[1].push(b1.0.min(b2.0));
            c[2].push(a1.1.min(a2.1)); c[3].push(b1.1.min(b2.1));
            c[4].push(a1.2.min(a2.2)); c[5].push(b1.2.min(b2.2));
            c[6].push(a1.3.min(a2.3)); c[7].push(b1.3.min(b2.3));
        }
        let m:Vec<f64>=(0..8).map(|i|med(&mut c[i])).collect();
        for i in 0..8 { t[i]+=m[i]; }
        let d=|a:f64,b:f64| if a>0.0 {100.0*(b-a)/a} else {0.0};
        println!("| {id} | {:.2} | {:.2} | {:+.1} | {:.2} | {:.2} | {:+.1} | {:.2} | {:.2} | {:+.1} | {:+.1} |",
            m[0]/1e6,m[1]/1e6,d(m[0],m[1]), m[2]/1e6,m[3]/1e6,d(m[2],m[3]),
            m[4]/1e6,m[5]/1e6,d(m[4],m[5]), d(m[6],m[7]));
    }
    let d=|a:f64,b:f64| if a>0.0 {100.0*(b-a)/a} else {0.0};
    println!("| **board** | **{:.1}** | **{:.1}** | **{:+.1}** | **{:.1}** | **{:.1}** | **{:+.1}** | **{:.1}** | **{:.1}** | **{:+.1}** | **{:+.1}** |",
        t[0]/1e6,t[1]/1e6,d(t[0],t[1]), t[2]/1e6,t[3]/1e6,d(t[2],t[3]),
        t[4]/1e6,t[5]/1e6,d(t[4],t[5]), d(t[6],t[7]));
}
