//! N2 instrument harvest: the X2 fire rate. N1's whole value is this ratio.
//! Also decodes C-zstd frames -- provenance is content (codec-measurement §9),
//! and N21 already showed a 20x provenance swing on this codec.
const IDS:&[&str]=&["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice","jsonlog-16m","text-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap=8usize<<20;
    println!("{:<10}{:>12}{:>12}{:>10}","level","X2 builds","X2 uses","used%");
    for lvl in [1i32,3,9,19] {
        let _=rusty_zstd::take_x2_stats();
        for id in IDS { let Some(f)=load(id) else{continue}; let src=&f[..f.len().min(cap)];
            let z=rusty_zstd::compress(src,lvl).unwrap();
            let o=rusty_zstd::decompress(&z).unwrap(); assert_eq!(o,src); }
        let (b,u)=rusty_zstd::take_x2_stats();
        println!("{:<10}{:>12}{:>12}{:>9.1}%",format!("ours L{lvl}"),b,u,
            if b>0 {100.0*u as f64/b as f64} else {0.0});
    }
    // foreign encoder
    if let Some(dir)=std::env::args().nth(1) {
        let _=rusty_zstd::take_x2_stats();
        let mut fs: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e|e.ok().map(|e|e.path()))
            .filter(|p|p.extension().map(|e|e=="zst").unwrap_or(false)).collect();
        fs.sort();
        for f in &fs { let z=std::fs::read(f).unwrap(); let _=rusty_zstd::decompress(&z).expect("C frame"); }
        let (b,u)=rusty_zstd::take_x2_stats();
        println!("{:<10}{:>12}{:>12}{:>9.1}%","C zstd",b,u,if b>0 {100.0*u as f64/b as f64} else {0.0});
    }
}
