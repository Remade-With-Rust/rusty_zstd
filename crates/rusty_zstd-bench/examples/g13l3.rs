//! GATE 13 @ L3. Step 1: is it dead? `push_literals` has ONE call site
//! (find_fast's match commit), so toggling the arm must do nothing at L3.
//! Step 2: size the opportunity -- mean literal run per sequence, which decides
//! whether a fixed-width 16-byte copy can serve it.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("=== is the gate dead at L{lvl}? ===");
    let mut moved=0;
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_litpush_arm(true);
        let a=rusty_zstd::compress(src,lvl).unwrap();
        rusty_zstd::set_litpush_arm(false);
        let b=rusty_zstd::compress(src,lvl).unwrap();
        rusty_zstd::set_litpush_arm(true);
        if a!=b {moved+=1;}
    }
    println!("corpora whose OUTPUT moves with the arm: {moved}/18 (expected 0 -- it is byte-identical by design)");

    println!("\n=== the opportunity: literal runs per sequence at L{lvl} ===");
    println!("{:<14}{:>12}{:>14}{:>12}{:>10}","corpus","sequences","literal bytes","lit/seq","<=16?");
    let (mut ts,mut tl)=(0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_dfast_match_stats();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (mb,sq,bb,_rb,_rh)=rusty_zstd::take_dfast_match_stats();
        if sq==0 {continue;}
        let lit=bb.saturating_sub(mb);
        let per=lit as f64/sq as f64;
        ts+=sq; tl+=lit;
        println!("{id:<14}{sq:>12}{lit:>14}{per:>12.2}{:>10}",if per<=16.0 {"yes"} else {"NO"});
    }
    println!("\nTOTAL {ts} sequences, {tl} literal bytes, mean {:.2} bytes/seq", tl as f64/ts.max(1) as f64);
    println!("Each sequence performs ONE runtime-length extend_from_slice the");
    println!("compiler cannot lower to a constant-width move.");
}
