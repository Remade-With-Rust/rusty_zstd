//! Why is GATE 12 dead on zeros / text-32m / incomp-32m / x-ray at L1?
//! Hypothesis: the search barely RUNS on them -- GATE 16's raw short-circuit
//! and degenerate match structure mean there is no back-fill to gate.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","x-ray","dickens","mr"];
fn main(){
    println!("{:<14}{:>9}{:>13}{:>13}{:>10}","corpus","size KB","pos (G16 on)","pos (G16 off)","ratio");
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_incomp_skip_arm(None);
        let _=rusty_zstd::take_mm();
        let sz=rusty_zstd::compress(src,1).unwrap().len();
        let on=rusty_zstd::take_mm().0;
        rusty_zstd::set_incomp_skip_arm(Some(false));
        let _=rusty_zstd::take_mm();
        let _=rusty_zstd::compress(src,1).unwrap();
        let off=rusty_zstd::take_mm().0;
        rusty_zstd::set_incomp_skip_arm(None);
        println!("{id:<14}{:>9}{on:>13}{off:>13}{:>10.1}x",sz/1024,
            if on>0 {off as f64/on as f64} else {f64::INFINITY});
    }
    println!("\n2 MiB input = 2,097,152 positions if every one were visited");
}
