//! GATE 9 @ L19/L22 protocol step 1: is `step0` dead here? L19/L22 run BtUltra2
//! -> find_opt, which advances `i += 1` and calls bt_find_best at every position.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for &lvl in &[19i32,22] {
        let mut moved=0;
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_step0_arm(2);
            let a=rusty_zstd::compress(src,lvl).unwrap().len();
            rusty_zstd::set_step0_arm(1);
            let b=rusty_zstd::compress(src,lvl).unwrap().len();
            rusty_zstd::set_step0_arm(4);
            let c=rusty_zstd::compress(src,lvl).unwrap().len();
            if a!=b||a!=c { moved+=1; }
        }
        rusty_zstd::set_step0_arm(2);
        println!("L{lvl}: step0 in {{1,2,4}} moves {moved}/18 sizes{}", if moved==0 {"  -> GATE 9 IS DEAD HERE"} else {""});
    }
}
