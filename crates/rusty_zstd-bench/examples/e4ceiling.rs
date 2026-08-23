//! E4 ceiling probe: how many positions do the prime/fill paths actually hash?
//! Deterministic counts, decided before anything is built.
const IDS: &[&str] = &["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap: usize = std::env::var("E4_CAP").ok().and_then(|s|s.parse().ok()).unwrap_or(8<<20);
    println!("{:<8}{:>16}{:>16}{:>14}", "level", "prime_iters", "input bytes", "iters/MiB");
    for lvl in [1i32,3,5,9,12,19,22] {
        let _ = rusty_zstd::take_prime_iters();
        let mut total_in = 0usize;
        for id in IDS {
            let Some(f)=load(id) else {continue};
            let src=&f[..f.len().min(cap)];
            total_in += src.len();
            let _ = rusty_zstd::compress(src, lvl).unwrap();
        }
        let it = rusty_zstd::take_prime_iters();
        println!("{:<8}{:>16}{:>16}{:>14.0}", format!("L{lvl}"), it, total_in,
            it as f64 / (total_in as f64 / (1<<20) as f64));
    }
    println!("\nprime_tables only runs when a payload_off/prefix exists (MT overlap, dict, streaming).");
}
