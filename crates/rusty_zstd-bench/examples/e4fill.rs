//! E4 ceiling probe, part 2: how many positions do the FILL paths hash?
//! E4 proposes a vector tile; a tile needs positions per call, not just calls.
const IDS: &[&str] = &["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap = 8usize<<20;
    println!("{:<7}{:>16}{:>14}{:>14}{:>14}{:>12}", "level","lazy_fill(a,b,c)","","","dfast_fill","opt_fill_ins");
    for lvl in [1i32,3,5,9,12,19,22] {
        let _=rusty_zstd::take_lazy_fill(); let _=rusty_zstd::take_dfast_fill(); let _=rusty_zstd::take_opt_fill_ins();
        let mut n=0usize;
        for id in IDS {
            let Some(f)=load(id) else {continue};
            let src=&f[..f.len().min(cap)]; n+=src.len();
            let _=rusty_zstd::compress(src,lvl).unwrap();
        }
        let l=rusty_zstd::take_lazy_fill();
        println!("{:<7}{:>16}{:>14}{:>14}{:>14}{:>12}   ({} MiB in)",
            format!("L{lvl}"), l.0, l.1, l.2,
            rusty_zstd::take_dfast_fill(), rusty_zstd::take_opt_fill_ins(), n>>20);
    }
}
