//! GATE 16, one layer deeper: the gate's signal is BINARY. Is the MARGIN by
//! which a block missed compressing a better one? If mozilla's isolated raw
//! blocks barely miss while incomp's runs miss by a mile, a margin threshold
//! could skip after ONE raw block where that is safe.
const IDS:&[&str]=&["incomp-32m","mozilla","x-ray","sao","osdb","ooffice","jsonlog-16m","mr","samba","webster"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("L{lvl}: for blocks that went RAW, payload/raw_limit\n");
    println!("{:<14}{:>9}{:>10}{:>10}{:>10}{:>12}","corpus","raw blks","exit0","exit1","exit2","margin@e2");
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_raw_skip_arm(false);   // search every block, so every
                                               // block's margin is real
        let _=rusty_zstd::take_raw_margin();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (sum,n,_h)=rusty_zstd::take_raw_margin();
        let e=rusty_zstd::take_raw_exits();
        let tot=e[0]+e[1]+e[2];
        if tot==0 {continue;}
        println!("{id:<14}{tot:>9}{:>10}{:>10}{:>10}{:>12}",e[0],e[1],e[2],
            if n>0 {format!("{:.4}",sum as f64/n as f64/1000.0)} else {"-".into()});
    }
    rusty_zstd::set_raw_skip_arm(true);
}
