//! LEVEL MONOTONICITY AUDIT. `higher_level_never_larger_osdb` gates one corpus
//! and stops at L19, so L20-L22 are ungated. A higher level emitting MORE bytes
//! is a user-visible defect.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const LEVELS: &[i32] = &[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22];
fn main(){
    let mut viol=0; let mut flat=0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        let mut sizes=vec![];
        for &l in LEVELS { sizes.push(rusty_zstd::compress(src,l).unwrap().len()); }
        // report every level that is LARGER than some lower level
        let mut worst=String::new();
        for i in 1..sizes.len() {
            let best_below=*sizes[..i].iter().min().unwrap();
            if sizes[i] > best_below {
                viol+=1;
                worst.push_str(&format!(" L{}(+{})", LEVELS[i], sizes[i]-best_below));
            }
        }
        let hi_flat = sizes[18]==sizes[21]; // L19 vs L22
        if hi_flat { flat+=1; }
        println!("{id:<14} L19 {:>9}  L22 {:>9}{}{}", sizes[18], sizes[21],
            if hi_flat {"  [L19==L22]"} else {"             "},
            if worst.is_empty() {String::new()} else {format!("  VIOLATIONS:{worst}")});
    }
    println!("\n{viol} monotonicity violations across 18 corpora x 22 levels");
    println!("{flat}/18 corpora produce IDENTICAL output at L19 and L22");
}
