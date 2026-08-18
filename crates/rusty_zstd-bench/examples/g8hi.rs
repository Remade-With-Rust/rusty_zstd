//! GATE 8 @ L19/L22 protocol step 1: is the gate DEAD here? `pipe_enabled()` is
//! consumed only by `find_fast`; L19/L22 run BtUltra2 -> find_opt. Size alone
//! cannot answer it (a byte-identical gate reads 0/18 either way), so pair it
//! with the finder call counts.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for &lvl in &[19i32, 22] {
        let (mut moved, mut fast, mut opt, mut dfast, mut ffpipe)=(0,0u64,0u64,0u64,0u64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            let _=rusty_zstd::take_finder_calls(); let _=rusty_zstd::take_dfast_calls();
            let _=rusty_zstd::take_ff_pipe();
            rusty_zstd::set_pipe_arm(true);
            let a=rusty_zstd::compress(src,lvl).unwrap().len();
            let (fc,oc)=rusty_zstd::take_finder_calls();
            let (ds,dr)=rusty_zstd::take_dfast_calls();
            let (fp,_,_)=rusty_zstd::take_ff_pipe();
            rusty_zstd::set_pipe_arm(false);
            let b=rusty_zstd::compress(src,lvl).unwrap().len();
            if a!=b { moved+=1; }
            fast+=fc; opt+=oc; dfast+=ds+dr; ffpipe+=fp;
        }
        rusty_zstd::set_pipe_arm(true);
        println!("L{lvl}: {moved}/18 sizes move | find_fast {fast} | find_opt {opt} | find_dfast {dfast} | pipelined blocks {ffpipe}");
        println!("      -> {}\n", if fast==0 {"GATE 8 IS DEAD HERE (no find_fast caller)"} else {"alive"});
    }
}
