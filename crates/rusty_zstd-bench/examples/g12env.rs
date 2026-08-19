//! GATE 12 @ L19: price the per-jump env lookup DETERMINISTICALLY. L19 timing is
//! inadmissible on this box (nulls of +11.05% / -8.69% observed, against the
//! recorded +-43% L19 self-noise), so: exact count x isolated per-op cost.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    // 1. isolated cost of one std::env::var miss -- the shape the guard performed
    let t=std::time::Instant::now();
    let n=200_000;
    let mut acc=0usize;
    for _ in 0..n { acc+=std::env::var("RZSTD_OPT_FILL_S").map(|v|v.len()).unwrap_or(0); }
    let ns=t.elapsed().as_secs_f64()*1e9/n as f64;
    println!("std::env::var (miss): {ns:.1} ns/call  [acc {acc}]\n");

    // 2. exact number of guard evaluations removed
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    println!("{:<14}{:>13}{:>13}{:>12}","corpus","DP positions","jumps","est. ms saved");
    let (mut tp,mut tj)=(0u64,0u64);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_opt_skips();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (pos,_inf,_jump,jumps)=rusty_zstd::take_opt_skips();
        tp+=pos; tj+=jumps;
        println!("{id:<14}{pos:>13}{jumps:>13}{:>12.2}",jumps as f64*ns/1e6);
    }
    println!("\nTOTAL DP positions {tp}, jumps {tj}");
    println!("at >=1 env lookup per jump: {:.0} lookups removed = {:.1} ms of pure overhead",
        tj as f64, tj as f64*ns/1e6);
}
