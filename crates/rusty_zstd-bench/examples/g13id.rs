fn main(){
    let ids=["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","versions-16m","incomp-32m","zeros-32m"];
    let mut bad=0;
    for id in ids{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        for lvl in [3i32,4]{
            rusty_zstd::set_dfast_litpush_arm(false);
            let a=rusty_zstd::compress(src,lvl).unwrap();
            rusty_zstd::set_dfast_litpush_arm(true);
            let b=rusty_zstd::compress(src,lvl).unwrap();
            if a!=b {println!("MISMATCH {id} L{lvl}: {} vs {}",a.len(),b.len()); bad+=1;}
            assert_eq!(rusty_zstd::decompress(&b).unwrap(),src,"roundtrip {id} L{lvl}");
        }
    }
    println!("arm ON vs OFF at L3/L4: {bad} mismatches, roundtrip OK");
}
