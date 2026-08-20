//! GATE 18 @ L3, step 3: min_match at L3, where the SHIPPED value is 5 (not L1's
//! 7) and the sequence counter is DFast's, not find_fast's.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(m:u32)->(i64,u64,u64,Vec<i64>){
    let (mut sz,mut pos,mut seqs)=(0i64,0u64,0u64);
    let mut per=vec![];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let mut p=rusty_zstd::compression_params(3,Some(src.len() as u64)).unwrap();
        p.min_match=m;
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_match_stats();
        let z=rusty_zstd::compress_with_params(src,p,false).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip mls={m} {id}");
        sz+=z.len() as i64; per.push(z.len() as i64);
        pos+=rusty_zstd::take_mm().0;
        seqs+=rusty_zstd::take_dfast_match_stats().1;
    }
    (sz,pos,seqs,per)
}
fn main(){
    let (s0,p0,q0,base)=run(5);
    println!("L3 min_match, SHIPPED = 5: {s0} bytes, {p0} positions, {q0} sequences\n");
    println!("{:>5}{:>11}{:>10}{:>10}   worst corpus","mls","size %","pos %","seqs %");
    for m in [3u32,4,6,7,8]{
        let (s,p,q,per)=run(m);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            if k<per.len(){let d=100.0*(per[k]-base[k]) as f64/base[k].max(1) as f64;
                if d>w {w=d; wid=id;}}
        }
        println!("{m:>5}{:>10.4}%{:>9.2}%{:>9.2}%   {wid} {w:+.3}%",
            100.0*(s-s0) as f64/s0 as f64,
            100.0*(p as f64-p0 as f64)/p0 as f64,
            100.0*(q as f64-q0 as f64)/q0.max(1) as f64);
    }
}
