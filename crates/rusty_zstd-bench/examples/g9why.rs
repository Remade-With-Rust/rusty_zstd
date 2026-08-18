const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_finder_calls();
        rusty_zstd::set_step0_arm(2);
        let a=rusty_zstd::compress(src,19).unwrap().len();
        let (fast,opt)=rusty_zstd::take_finder_calls();
        rusty_zstd::set_step0_arm(4);
        let b=rusty_zstd::compress(src,19).unwrap().len();
        if a!=b { println!("  {id:<14} {a:>9} -> {b:>9}  ({:+.3}%)  find_fast {fast}, find_opt {opt}", 100.0*(b as f64-a as f64)/a as f64); }
    }
    rusty_zstd::set_step0_arm(2);
}
