const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for lvl in [22i32,19] {
        let (mut moved,mut delta)=(0,0i64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            std::env::set_var("RZSTD_OPT_FILL","0");
            let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            std::env::remove_var("RZSTD_OPT_FILL");
            let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            if a!=b { moved+=1; }
            delta+=b-a;
        }
        println!("L{lvl}: fill off->on moves {moved}/18 sizes, {delta:+} bytes  -> GATE 11 IS {}",
            if moved==0 {"DEAD"} else {"ALIVE (dispatched)"});
    }
}
