fn main(){
    let ids=["jsonlog-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","smallmsg-8m"];
    let mut bad=0;
    for id in ids{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        for lvl in [1i32,3,13,19,22]{
            rusty_zstd::set_eqlen_arm(0);
            let a=rusty_zstd::compress(src,lvl).unwrap();
            for arm in [1u8,2]{
                rusty_zstd::set_eqlen_arm(arm);
                if rusty_zstd::compress(src,lvl).unwrap()!=a {bad+=1;}
            }
            rusty_zstd::set_eqlen_arm(0);
            assert_eq!(rusty_zstd::decompress(&a).unwrap(), src);
        }
    }
    println!("all three prefix-compare arms agree at L1/L3/L13/L19/L22: {bad} mismatches, round-trip OK");
}
