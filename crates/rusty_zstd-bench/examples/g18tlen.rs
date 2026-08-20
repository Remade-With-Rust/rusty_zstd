//! GATE 18 @ L1, step 3: `target_length` is the L1 level constant that is live.
//! It gates early_raw_skip (GATE 16's otherwise-dead mechanism) AND sets the
//! search step s0 = tlen + 1. Shipped default is 0. This is zstd's --fast=N.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(t:u32)->(i64,u64,u64,Vec<i64>){
    let (mut sz,mut pos,mut raw)=(0i64,0u64,0u64);
    let mut per=vec![];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let mut p=rusty_zstd::compression_params(1,Some(src.len() as u64)).unwrap();
        p.target_length=t;
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_raw_exits();
        let z=rusty_zstd::compress_with_params(src,p,false).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip tlen={t} {id}");
        sz+=z.len() as i64; per.push(z.len() as i64);
        pos+=rusty_zstd::take_mm().0;
        let e=rusty_zstd::take_raw_exits(); raw+=e[0]+e[1]+e[2];
    }
    (sz,pos,raw,per)
}
fn main(){
    let (s0,p0,r0,base)=run(0);
    println!("L1 target_length, SHIPPED = 0: {s0} bytes, {p0} positions, {r0} raw blocks\n");
    println!("{:>6}{:>12}{:>10}{:>11}{:>10}   worst corpus","tlen","size","size %","positions","pos %");
    for t in [1u32,2,3,4,5,7]{
        let (s,p,r,per)=run(t);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            if k<per.len(){let d=100.0*(per[k]-base[k]) as f64/base[k].max(1) as f64;
                if d>w {w=d; wid=id;}}
        }
        println!("{t:>6}{s:>12}{:>9.3}%{p:>11}{:>9.2}%   {wid} {w:+.2}%  (raw {r})",
            100.0*(s-s0) as f64/s0 as f64,
            100.0*(p as f64-p0 as f64)/p0 as f64);
    }
}
