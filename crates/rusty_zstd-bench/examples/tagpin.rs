//! Is the Gate 7 divergence the FILTER, or the per-block TOGGLING of it?
//!   A  ut pinned TRUE   (RZSTD_TAG_T = -1, so tag_yield >= min always)
//!   B  ut pinned FALSE  (no tags array at all)
//! Zero false rejects were measured at all three filter sites, so if the arms
//! are equivalent A must equal B.
const IDS: &[&str] = &["sao","mozilla","samba","x-ray","mr","nci","dickens","webster"];
fn main(){
    let mut diff=0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        std::env::set_var("RZSTD_TAG_T","-1");
        rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(true);
        let a=rusty_zstd::compress(src,1).unwrap();
        std::env::remove_var("RZSTD_TAG_T");
        rusty_zstd::set_tag_alloc_arm(false); rusty_zstd::set_tag_arm(false);
        let b=rusty_zstd::compress(src,1).unwrap();
        // and today's DISPATCHED behaviour, for reference
        std::env::remove_var("RZSTD_TAG_T");
        rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(true);
        let c=rusty_zstd::compress(src,1).unwrap();
        let tag = if a==b {"same"} else {"DIFF"};
        if a!=b { diff+=1; }
        println!("{id:<10} pinnedON {:>9}  pinnedOFF {:>9} [{tag}]  dispatched {:>9}{}",
            a.len(), b.len(), c.len(), if c==b {""} else {"  <- dispatch differs"});
    }
    println!("\n{diff}/8 differ with ut PINNED.");
    println!("0 => the arms are equivalent and the divergence is the TOGGLING.");
    rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(true);
}
