//! Exhaustive coverage scan of the bt specialisation across the SIZE axis --
//! the axis whose omission left 24 of 64 cells on the runtime body.
const PAIRS:&[(u32,u32)]=&[(11,11),(12,12),(13,13),(14,14),(14,15),(15,15),(16,16),(17,17),(17,18),(18,18),(19,18),(19,19),(20,20),(21,21),(22,22),(22,23),(22,24),(23,22),(23,23),(23,24),(24,24)];
fn main(){
    let mut miss=std::collections::BTreeMap::new();
    let mut tot=0; let mut m=0;
    let mut n=1024usize;
    while n <= (64<<20) {
        for lvl in [13i32,14,15,16,17,18,19,20,21,22]{
            let p=rusty_zstd::compression_params(lvl,Some(n as u64)).unwrap();
            let k=(p.hash_log.min(24),p.chain_log.min(24));
            tot+=1;
            if !PAIRS.contains(&k){ m+=1; *miss.entry(k).or_insert(0usize)+=1; }
        }
        n = n + (n/4).max(1024);
    }
    println!("scanned {tot} (size, level) cells from 1 KiB to 64 MiB, step +25%");
    if miss.is_empty(){ println!("MISSES: none -- the specialisation covers every cell"); }
    else { println!("{m} misses, by pair:"); for (k,v) in &miss { println!("   {:?} x{}",k,v); } }
}
