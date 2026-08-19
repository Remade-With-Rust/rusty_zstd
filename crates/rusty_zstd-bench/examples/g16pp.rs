const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let run=|pp:u32|{
        rusty_zstd::set_raw_skip_arm(true); rusty_zstd::set_raw_probe_arm(pp);
        let (mut s,mut q)=(0i64,0u64);
        for id in IDS{
            let Some(f)=load(id) else{continue};
            let src=&f[..f.len().min(8<<20)];
            let _=rusty_zstd::take_mm();
            s+=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            q+=rusty_zstd::take_mm().0;
        }
        (s,q)
    };
    let (s0,q0)=run(16);
    println!("{:>8}{:>13}{:>11}{:>14}{:>10}","period","size","size %","positions","pos %");
    println!("{:>8}{s0:>13}{:>11}{q0:>14}{:>10}",16,"(shipped)","-");
    for pp in [1u32,2,3,4,6,8]{
        let (s,q)=run(pp);
        println!("{pp:>8}{s:>13}{:>10.4}%{q:>14}{:>9.2}%",
            100.0*(s-s0) as f64/s0 as f64, 100.0*(q as f64-q0 as f64)/q0 as f64);
    }
    rusty_zstd::set_raw_probe_arm(0);
}
