//! Does Gate 7's tag compare reject candidates `fast_probe` would ACCEPT?
//! If yes, the recorded "byte-identical" verdict is false and it costs ratio.
const IDS: &[&str] = &["sao","mozilla","samba","x-ray","mr","dickens","nci","webster"];
fn main(){
    println!("{:<12}{:>14}{:>16}{:>10}", "corpus","tag rejects","FALSE rejects","share");
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(true);
        let _=rusty_zstd::take_tag_rejects();
        let _=rusty_zstd::compress(src,1).unwrap();
        let (f,t)=rusty_zstd::take_tag_rejects();
        println!("{id:<12}{t:>14}{f:>16}{:>9.2}%", 100.0*f as f64/t.max(1) as f64);
    }
    println!("\nA FALSE reject = the tag said no, but the real 4-byte compare said YES.");
    println!("Any non-zero count means the filter is NOT byte-identical.");
}
