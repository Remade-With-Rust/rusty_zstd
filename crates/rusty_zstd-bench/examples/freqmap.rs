//! WHERE DOES THE COUNT MULTIPLY? The deterministic map that picks the next
//! SIMD target, per the SIMD-1/SIMD-2 law: an ISA win only pays where the
//! improved code actually RUNS. Pure counts, no clock.
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap = 8usize<<20;
    println!("| level | probes/MiB (CHAINED, serial) | **fills/MiB (INDEPENDENT)** | seqs/MiB | lit bytes/MiB | fills vs DecSeq |");
    println!("| ----: | ---------: | -------: | -------: | ------------: | ----: |");
    for lvl in [1i32,3,5,7,9,12,13,16,19,22] {
        let (mut pr, mut sq, mut lb, mut mib, mut fi) = (0u64,0u64,0u64,0f64,0u64);
        for id in IDS {
            let Some(f)=load(id) else{continue};
            let s=&f[..f.len().min(cap)];
            rusty_zstd::prof_reset();
            let _=rusty_zstd::compress(s,lvl).unwrap();
            let c=rusty_zstd::prof_encode_counts();
            pr+=c.hash_probes; sq+=c.seqs; lb+=c.lit_bytes; fi+=c.hash_fills;
            mib+=s.len() as f64/1_048_576.0;
        }
        let ppm = pr as f64/mib; let spm = sq as f64/mib;
        let fpm = fi as f64/mib;
        println!("| L{lvl} | {:.0} | **{:.0}** | {:.0} | {:.0} | **{:.0}x** |",
            ppm, fpm, spm, lb as f64/mib, if spm>0.0 {fpm/spm} else {0.0});
    }
}
