//! Gate 7 is recorded as BYTE-IDENTICAL. Is it? Compare tag machinery fully on
//! vs fully absent, all 18, and name every divergence.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut bad=0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        // arm A: array present, filter ON (today)
        rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(true);
        let a=rusty_zstd::compress(src,lvl).unwrap();
        // arm B: array present, filter OFF  (isolates the FILTER)
        rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(false);
        let b=rusty_zstd::compress(src,lvl).unwrap();
        // arm C: no array at all
        rusty_zstd::set_tag_alloc_arm(false); rusty_zstd::set_tag_arm(false);
        let c=rusty_zstd::compress(src,lvl).unwrap();
        let f = if a.len()!=b.len() || a!=b {"FILTER"} else {"."};
        let s = if b.len()!=c.len() || b!=c {"ARRAY"} else {"."};
        if f!="." || s!="." {
            bad+=1;
            println!("{id:<14} on {:>9}  filterOff {:>9} ({f})  noArray {:>9} ({s})",
                a.len(), b.len(), c.len());
        }
    }
    println!("\n{bad}/18 diverge. FILTER = the tag compare changes output (should be impossible).");
    println!("               ARRAY  = merely ALLOCATING the array changes output.");
    rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(true);
}
