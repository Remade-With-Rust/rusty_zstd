//! GATE 16 @ L3. The shipped mechanism is the raw short circuit in
//! `encode_block`: after RAW_RUN_MIN consecutive raw blocks, skip the search and
//! emit literals, re-probing every RAW_PROBE_PERIOD blocks.
//! Step 1: does the default differ from the value set?
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("L{lvl}: arm ON (shipped) vs OFF (always search)\n");
    println!("{:<14}{:>11}{:>11}{:>10}{:>13}{:>10}","corpus","on","off","size %","mainloop pos","pos %");
    let (mut ta,mut tb,mut pa,mut pb)=(0i64,0i64,0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_incomp_skip_arm(Some(true));
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let xa=rusty_zstd::take_mm().0;
        rusty_zstd::set_incomp_skip_arm(Some(false));
        let _=rusty_zstd::take_mm();
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let xb=rusty_zstd::take_mm().0;
        rusty_zstd::set_incomp_skip_arm(None);
        ta+=a; tb+=b; pa+=xa; pb+=xb;
        if a!=b || xa!=xb {
            println!("{id:<14}{a:>11}{b:>11}{:>9.3}%{xa:>13}{:>9.1}%",
                100.0*(b-a) as f64/a as f64,
                if xb>0 {100.0*(xa as f64-xb as f64)/xb as f64} else {0.0});
        }
    }
    println!("\nTOTAL size {ta} vs {tb} ({:+.4}%)   mainloop positions {pa} vs {pb} ({:+.2}%)",
        100.0*(ta-tb) as f64/tb as f64,
        if pb>0 {100.0*(pa as f64-pb as f64)/pb as f64} else {0.0});
}
