//! find_fast_impl broken open: what is a POSITION made of at L1?
//! Deterministic counts first; the clock appears only to close the arithmetic
//! (ns/position -> implied IPC against the asm instruction count), never to
//! decide anything.
use std::time::Instant;
const IDS: &[&str] = &["dickens","reymont","webster","samba","sao","mr","jsonlog-16m","nci"];
fn main() {
    let cap = 8 << 20;
    println!("{:<12} {:>10} {:>11} {:>9} {:>7} {:>9} {:>9} {:>8} {:>9}",
        "corpus","bytes","positions","pos/B","probes/pos","hit%","spec use%","ns/B","ns/pos");
    for id in IDS {
        let Ok(f)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s=&f[..f.len().min(cap)];
        // counters
        rusty_zstd::prof_reset();
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_ff_pipe();
        let _=rusty_zstd::compress(s,1).unwrap();
        let (pos,_miss)=rusty_zstd::take_mm();
        let (_blk,sm,su)=rusty_zstd::take_ff_pipe();
        let arms=rusty_zstd::take_ff_arms();
        let c=rusty_zstd::prof_encode_counts();
        // clock: best-of-9, closure only
        let mut best=f64::MAX;
        for _ in 0..9 {
            let t=Instant::now();
            let z=rusty_zstd::compress(s,1).unwrap();
            let e=t.elapsed().as_secs_f64();
            std::hint::black_box(z.len());
            if e<best {best=e}
        }
        let nsb=best*1e9/s.len() as f64;
        println!("{:<12} arms spec={} TAG-GENERIC={} rep-generic={} other={}", id, arms[0], arms[1], arms[2], arms[3]);
        println!("{:<12} {:>10} {:>11} {:>9.3} {:>7.2} {:>8.1}% {:>8.1}% {:>8.2} {:>9.2}",
            id, s.len(), pos, pos as f64/s.len() as f64,
            c.hash_probes as f64/pos.max(1) as f64,
            c.probe_hits as f64/c.hash_probes.max(1) as f64*100.0,
            su as f64/sm.max(1) as f64*100.0,
            nsb, nsb*s.len() as f64/pos.max(1) as f64);
    }
    println!("\nNOTE: the profile build itself inflates ns; use ns/pos only for IPC closure.");
}
