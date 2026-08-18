//! Exhaustive determinism sweep: every corpus, every level, each frame preceded
//! by a DIFFERENT one so any cross-frame state leak shows up.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(256<<10);(*id,f[..n].to_vec())})).collect();
    let mut bad=0; let mut checked=0;
    for lvl in [1i32,2,3,5,7,9,12,13,16,18,19,22] {
        // reference: each corpus compressed from a fresh-ish state
        let want: Vec<Vec<u8>> = srcs.iter().map(|(_,s)| rusty_zstd::compress(s,lvl).unwrap()).collect();
        // now interleave: compress every OTHER corpus before re-checking each one
        for (i,(id,s)) in srcs.iter().enumerate() {
            for (j,(_,o)) in srcs.iter().enumerate() {
                if j==i { continue; }
                let _=rusty_zstd::compress(o,lvl).unwrap();
            }
            let got=rusty_zstd::compress(s,lvl).unwrap();
            checked+=1;
            if got!=want[i] {
                bad+=1;
                println!("  L{lvl} {id}: {} vs {}", want[i].len(), got.len());
            }
        }
    }
    println!("\n{bad} non-deterministic of {checked} (corpus x level, each preceded by all 17 others)");
}
