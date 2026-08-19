//! GATE 17 @ L1. The gate is a find_opt level constant and find_opt is L16+.
//! Step 1: prove it is unreachable at L1 rather than assert it.
const IDS:&[&str]=&["jsonlog-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","incomp-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("find_opt reachability by level (bt walk calls are its proxy)\n");
    println!("{:>6}{:>10}{:>16}{:>16}","level","strategy","bt spec calls","bt runtime");
    for lvl in [1i32,2,3,5,7,13,16,19]{
        let (mut a,mut b)=(0u64,0u64);
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(1<<20)];
            let _=rusty_zstd::take_bt_calls();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            let (x,y)=rusty_zstd::take_bt_calls(); a+=x; b+=y;
        }
        let p=rusty_zstd::compression_params(lvl,Some(1<<20)).unwrap();
        println!("{lvl:>6}{:>10}{a:>16}{b:>16}",format!("{:?}",p.strategy));
    }
    println!("\n=== the L1 LEVEL constants, for comparison ===");
    println!("{:>6}{:>10}{:>10}{:>10}{:>10}","level","minmatch","slog","tlen","strategy");
    for lvl in [1i32,2,3,4,5]{
        let p=rusty_zstd::compression_params(lvl,Some(8<<20)).unwrap();
        println!("{lvl:>6}{:>10}{:>10}{:>10}{:>10}",p.min_match,p.search_log,p.target_length,format!("{:?}",p.strategy));
    }
}
