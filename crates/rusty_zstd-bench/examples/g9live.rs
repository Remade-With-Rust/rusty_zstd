//! GATE 9 (`step0`, probe density) protocol step 1: is it DEAD at L3?
//! L3 runs DFast, whose advance is `ip += 1 + ((ip-anchor) >> 8)` -- no step0.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for &lvl in &[3i32, 1] {
        let mut moved=0; let mut detail=String::new();
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(4<<20)];
            rusty_zstd::set_step0_arm(2);
            let a=rusty_zstd::compress(src,lvl).unwrap().len();
            rusty_zstd::set_step0_arm(1);
            let b=rusty_zstd::compress(src,lvl).unwrap().len();
            rusty_zstd::set_step0_arm(4);
            let c=rusty_zstd::compress(src,lvl).unwrap().len();
            if a!=b || a!=c { moved+=1; detail.push_str(&format!(" {id}")); }
        }
        rusty_zstd::set_step0_arm(2);
        println!("L{lvl}: step0 in {{1,2,4}} moves {moved}/18 sizes{}", if moved==0 {"  -> GATE 9 IS DEAD HERE".into()} else {format!(" ->{detail}")});
    }
}
