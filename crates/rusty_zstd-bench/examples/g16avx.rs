//! "AVX2 faster on bigger files, disable on smaller" -- test it.
//! has_avx2() caches after one cpuid, so the per-FILE cost is one detection per
//! PROCESS, not per file. The per-CALL economics (4.60: tie under 8 bytes, 3x
//! above) do not depend on file size. Measure whether file size changes it.
fn main(){
    let Ok(full)=std::fs::read("corpora/data/silesia/webster")
        .or_else(|_|std::fs::read("corpora/data/generated/jsonlog-16m")) else{return};
    println!("L3 encode, AVX2 (arm 0) vs word loop (arm 1), by INPUT SIZE\n");
    println!("{:>10}{:>12}{:>12}{:>11}{:>9}","size","avx2 ms","words ms","words %","calls");
    for kb in [4usize, 16, 64, 256, 1024, 4096] {
        let n=(kb*1024).min(full.len());
        let src=&full[..n];
        let reps = (2_000_000usize / n.max(1)).clamp(3, 400);
        let mut best=[f64::MAX;2];
        let mut calls=0u64;
        for arm in [0u8,1]{
            for _ in 0..7 {
                rusty_zstd::set_eqlen_arm(arm);
                let _=rusty_zstd::take_eqlen_stats();
                let t=std::time::Instant::now();
                for _ in 0..reps { let _=rusty_zstd::compress(src,3).unwrap(); }
                let e=t.elapsed().as_secs_f64()*1000.0/reps as f64;
                if arm==0 { calls=rusty_zstd::take_eqlen_stats().0/reps.max(1) as u64; }
                if e<best[arm as usize] {best[arm as usize]=e;}
            }
        }
        println!("{:>9}K{:>12.3}{:>12.3}{:>10.1}%{:>9}",kb,best[0],best[1],
            100.0*(best[1]-best[0])/best[0], calls);
    }
    rusty_zstd::set_eqlen_arm(0);
}
