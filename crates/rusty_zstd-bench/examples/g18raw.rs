//! Is x-ray's mls=8 "speed win" just the file going RAW? If so it is not an
//! optimisation, it is abandonment -- and GATE 16 already gives that for free on
//! content that is genuinely incompressible.
fn main(){
    for id in ["x-ray","sao","dickens"]{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(8<<20)];
        println!("--- {id} ---");
        for m in [5u32,8]{
            let mut p=rusty_zstd::compression_params(3,Some(src.len() as u64)).unwrap();
            p.min_match=m;
            let _=rusty_zstd::take_raw_exits();
            let z=rusty_zstd::compress_with_params(src,p,false).unwrap();
            let e=rusty_zstd::take_raw_exits();
            let blocks=(src.len()+131071)/131072;
            println!("  mls={m}: {} bytes, raw blocks {}/{} ({:.0}% of the file emitted RAW)",
                z.len(), e[0]+e[1]+e[2], blocks,
                100.0*(e[0]+e[1]+e[2]) as f64/blocks as f64);
        }
    }
}
