//! GATE 17 @ L1, step 3: the gate is dead, so look at the L1 LEVEL CONSTANT that
//! plays its role. L1 ships min_match = 7 against L2's 6 and L3's 5, and it has
//! never been swept here. Lower = accept shorter matches (more sequences, better
//! coverage); higher = reject them.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("L{lvl}: sweeping min_match\n");
    println!("{:>5}{:>13}{:>11}{:>14}{:>10}   worst corpus","mls","size","size %","positions","pos %");
    let run=|m:u32|{
        let (mut sz,mut pos)=(0i64,0u64); let mut per=vec![];
        for id in IDS{
            let Some(f)=load(id) else{continue};
            let src=&f[..f.len().min(8<<20)];
            let mut p=rusty_zstd::compression_params(lvl,Some(src.len() as u64)).unwrap();
            p.min_match=m;
            let _=rusty_zstd::take_mm();
            let z=rusty_zstd::compress_with_params(src,p,false).unwrap();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip mls={m} {id}");
            sz+=z.len() as i64; per.push(z.len() as i64);
            pos+=rusty_zstd::take_mm().0;
        }
        (sz,pos,per)
    };
    let (s0,p0,base)=run(7);
    println!("{:>5}{s0:>13}{:>11}{p0:>14}{:>10}   (shipped)",7,"-","-");
    for m in [4u32,5,6,8,9]{
        let (s,p,per)=run(m);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            if k<per.len(){let d=100.0*(per[k]-base[k]) as f64/base[k].max(1) as f64;
                if d>w {w=d; wid=id;}}
        }
        println!("{m:>5}{s:>13}{:>10.4}%{p:>14}{:>9.2}%   {wid} {w:+.3}%",
            100.0*(s-s0) as f64/s0 as f64,
            100.0*(p as f64-p0 as f64)/p0 as f64);
    }
}
