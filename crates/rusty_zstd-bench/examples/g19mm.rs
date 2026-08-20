fn main(){
    let f=std::fs::read("corpora/data/silesia/dickens").unwrap();
    let s=&f[..f.len().min(8<<20)];
    for lv in [1,3,5,7]{
        let _=rusty_zstd::take_mm();
        let _=rusty_zstd::compress(s,lv).unwrap();
        let (t,m)=rusty_zstd::take_mm();
        println!("L{lv:<3} MM_TOTAL={t:<12} MM_MISS={m}");
    }
}
