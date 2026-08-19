//! How many positions does find_lazy's back-fill actually insert per site? If it
//! is 0 or 1, the stride knob cannot matter -- which is what 0/18 says.
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for lvl in [5i32,7,9,12] {
        let (mut f,mut ne,mut ins)=(0u64,0u64,0u64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            let _=rusty_zstd::take_lazy_fill();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            let (a,b,c)=rusty_zstd::take_lazy_fill();
            f+=a; ne+=b; ins+=c;
        }
        println!("L{lvl:<3} fill sites {f:>10} | with >=1 insert {ne:>10} ({:>5.1}%) | inserts {ins:>11} | mean per NON-EMPTY site {:.2}",
            100.0*ne as f64/f.max(1) as f64, ins as f64/ne.max(1) as f64);
    }
}
