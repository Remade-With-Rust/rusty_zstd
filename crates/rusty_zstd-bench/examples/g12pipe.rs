//! How far does the defect reach? `rep_len_ratio` is Gate 2 @ L1's second
//! dispatch variable. It starts at 1.0, the gate is `>= 1.0`, and the only code
//! that lowers it sits AFTER the pipelined loop's early return.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("{:<14}{:>9}{:>9}{:>10}","corpus","blocks","piped","piped %");
    let (mut tb,mut tp)=(0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_ff_pipe();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (piped,_,_)=rusty_zstd::take_ff_pipe();
        let blocks=((src.len()+131071)/131072) as u64;
        tb+=blocks; tp+=piped;
        println!("{id:<14}{blocks:>9}{piped:>9}{:>9.1}%",100.0*piped as f64/blocks as f64);
    }
    println!("\nTOTAL blocks {tb}, pipelined {tp} ({:.1}%) -- every one of these skips the rep_len_ratio update",
        100.0*tp as f64/tb as f64);
}
