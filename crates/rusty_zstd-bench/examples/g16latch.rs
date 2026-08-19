//! mozilla has ONE raw block, yet run_min=1 saved it 297,196 positions. That is
//! only possible if the gate SUSTAINS ITSELF: a skipped block is emitted raw,
//! which keeps raw_run high, which keeps skipping. Count the raw blocks.
const IDS:&[&str]=&["mozilla","incomp-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("L{lvl}: raw blocks emitted, by run_min\n");
    println!("{:<14}{:>10}{:>12}{:>12}{:>11}","corpus","run_min","raw blocks","size","size %");
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let mut base=0i64;
        for rm in [2u32,1]{
            rusty_zstd::set_raw_skip_arm(true);
            rusty_zstd::set_raw_run_min_arm(rm);
            let _=rusty_zstd::take_raw_exits();
            let z=rusty_zstd::compress(src,lvl).unwrap();
            let e=rusty_zstd::take_raw_exits();
            let n=z.len() as i64;
            if rm==2 {base=n;}
            println!("{id:<14}{rm:>10}{:>12}{n:>12}{:>10.4}%",e[0]+e[1]+e[2],
                100.0*(n-base) as f64/base as f64);
        }
    }
    rusty_zstd::set_raw_run_min_arm(0);
}
