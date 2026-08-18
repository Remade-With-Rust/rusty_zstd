//! FULL files, not 2 MiB slices: does the warm-up schedule still hold when a
//! frame has many more blocks than the warm-up covers?
const IDS: &[&str] = &["versions-16m","text-32m","jsonlog-16m","nci","mr","samba","mozilla","webster","x-ray","dickens"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    println!("{:<14}{:>10}{:>12}{:>12}{:>11}", "corpus","MiB","const ON","dispatched","delta");
    let (mut ta,mut tb)=(0usize,0usize); let (mut pa,mut pb)=(0u64,0u64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(16<<20)];
        std::env::set_var("RZSTD_OPT_REP_MIN","-1");
        let _=rusty_zstd::take_opt_rep();
        let a=rusty_zstd::compress(src,lvl).unwrap().len();
        let (p1,_,_)=rusty_zstd::take_opt_rep();
        std::env::remove_var("RZSTD_OPT_REP_MIN");
        let _=rusty_zstd::take_opt_rep();
        let z=rusty_zstd::compress(src,lvl).unwrap();
        let (p2,_,_)=rusty_zstd::take_opt_rep();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id}");
        println!("{id:<14}{:>10.1}{a:>12}{:>12}{:>10.3}%", src.len() as f64/1048576.0, z.len(),
            100.0*(z.len() as f64-a as f64)/a as f64);
        ta+=a; tb+=z.len(); pa+=p1; pb+=p2;
    }
    println!("\nTOTAL size {:+.4}%   rep probes {pa} -> {pb} ({:.1}% removed)",
        100.0*(tb as f64-ta as f64)/ta as f64, 100.0*(pa-pb) as f64/pa.max(1) as f64);
}
