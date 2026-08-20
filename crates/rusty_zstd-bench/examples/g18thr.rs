//! Sweep the probe's accept threshold. The proxy ranks ooffice correctly and
//! mozilla/sao incorrectly, so the question is what the best achievable
//! operating point is with a count-based signal.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(on:bool,thr:f32)->(i64,u64,Vec<i64>){
    rusty_zstd::set_step_probe_arm(on);
    rusty_zstd::set_step_forfeit_arm(thr);
    let (mut s,mut p)=(0i64,0u64); let mut per=vec![];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let _=rusty_zstd::take_mm();
        let z=rusty_zstd::compress(src,1).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src);
        s+=z.len() as i64; per.push(z.len() as i64); p+=rusty_zstd::take_mm().0;
    }
    (s,p,per)
}
fn main(){
    let (s0,p0,base)=run(false,0.5);
    println!("baseline (probe off): {s0} bytes, {p0} positions\n");
    println!("{:>9}{:>11}{:>14}{:>10}   worst corpus","thresh","size %","positions","pos %");
    for t in [0.0001f32,0.0005,0.001,0.002,0.005,0.5]{
        let (s,p,per)=run(true,t);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            if k<per.len(){let d=100.0*(per[k]-base[k]) as f64/base[k].max(1) as f64;
                if d>w {w=d; wid=id;}}
        }
        println!("{t:>9.4}{:>10.4}%{p:>14}{:>9.2}%   {} {:+.3}%",
            100.0*(s-s0) as f64/s0 as f64,
            100.0*(p as f64-p0 as f64)/p0 as f64,
            if wid.is_empty(){"none"}else{wid}, w);
    }
    rusty_zstd::set_step_forfeit_arm(0.5);
}
