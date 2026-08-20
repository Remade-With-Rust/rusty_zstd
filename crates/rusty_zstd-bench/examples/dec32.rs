//! Which decoder copy TIER actually fires? The 32B tier is exactly one ymm
//! register -- if it dominates, AVX2 is a one-instruction-pair swap there.
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("decoder copy tier engagement at L1 (32B tier = one ymm)\n");
    println!("{:<14}{:>12}{:>12}{:>12}{:>12}{:>9}","corpus","lit 32B","lit 16B","match 32B","match 16B","32B %");
    let (mut a,mut b,mut c,mut d)=(0u64,0u64,0u64,0u64);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let z=rusty_zstd::compress(src,1).unwrap();
        let _=rusty_zstd::take_dec_copies();
        let out=rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out.len(),src.len());
        let (l32,m32,l16,m16)=rusty_zstd::take_dec_copies();
        a+=l32;b+=l16;c+=m32;d+=m16;
        let t=l32+m32+l16+m16;
        println!("{id:<14}{l32:>12}{l16:>12}{m32:>12}{m16:>12}{:>8.1}%",
            if t>0 {100.0*(l32+m32) as f64/t as f64} else {0.0});
    }
    let t=a+b+c+d;
    println!("\nTOTAL 32B {} / {} = {:.1}% of all tiered copies",a+c,t,100.0*(a+c) as f64/t as f64);
}
