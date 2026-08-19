//! One number: total encode time for the corpus. Run from two BUILDS (baseline
//! vs -C target-feature=+avx2) to price AVX2 as a compile target rather than as
//! a runtime dispatch.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let mut srcs=vec![];
    for id in IDS{
        if let Ok(f)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) {
            srcs.push(f[..f.len().min(4<<20)].to_vec());
        }
    }
    let mut best=f64::MAX; let mut tot=0usize;
    for _ in 0..9 {
        let t=std::time::Instant::now();
        let mut n=0usize;
        for s in &srcs { n+=rusty_zstd::compress(s,lvl).unwrap().len(); }
        let e=t.elapsed().as_secs_f64()*1000.0;
        if e<best {best=e;} tot=n;
    }
    println!("L{lvl} encode: {best:.1} ms (best of 9), total {tot} bytes");
}
