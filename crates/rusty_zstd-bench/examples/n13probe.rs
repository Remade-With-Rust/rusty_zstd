//! N13 ceiling probe: is `huffman_nbits`'s O(n^2) tree merge worth replacing?
const IDS:&[&str]=&["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap=8usize<<20;
    println!("{:<8}{:>10}{:>10}{:>16}{:>14}","level","calls","mean n","sum n^2 (work)","per MiB");
    for lvl in [1i32,3,9,19] {
        let _=rusty_zstd::take_n13_stats();
        let mut b=0usize;
        for id in IDS { let Some(f)=load(id) else{continue}; let src=&f[..f.len().min(cap)]; b+=src.len();
            let _=rusty_zstd::compress(src,lvl).unwrap(); }
        let st=rusty_zstd::take_n13_stats();
        println!("{:<8}{:>10}{:>10.1}{:>16}{:>14.0}",format!("L{lvl}"),st[0],
            if st[0]>0 {st[1] as f64/st[0] as f64} else {0.0}, st[2],
            st[2] as f64/((b>>20) as f64));
    }
}
