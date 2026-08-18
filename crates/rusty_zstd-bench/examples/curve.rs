//! The level curve where monotonicity breaks. Strategy per level:
//! L1-2 Fast, L3-4 DFast, L5 Greedy, L6-7 Lazy, L8-12 Lazy2,
//! L13-15 BtLazy2, L16-17 BtOpt, L18 BtUltra, L19-22 BtUltra2.
fn main(){
    let ids: Vec<String> = std::env::args().skip(1).collect();
    let ids = if ids.is_empty() { vec!["x-ray".into(),"jsonlog-16m".into()] } else { ids };
    for id in ids {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        println!("\n=== {id} ({} MiB slice) ===", src.len()>>20);
        let mut best=usize::MAX; let mut best_l=0;
        for l in 1..=22 {
            let n=rusty_zstd::compress(src,l).unwrap().len();
            let strat = match l {1..=2=>"Fast",3..=4=>"DFast",5=>"Greedy",6..=7=>"Lazy",
                8..=12=>"Lazy2",13..=15=>"BtLazy2",16..=17=>"BtOpt",18=>"BtUltra",_=>"BtUltra2"};
            let mark = if n>best {format!("  <-- WORSE than L{best_l} by {}", n-best)} else {String::new()};
            if n<best {best=n; best_l=l;}
            println!("  L{l:<3}{strat:<9}{n:>10}{mark}");
        }
    }
}
