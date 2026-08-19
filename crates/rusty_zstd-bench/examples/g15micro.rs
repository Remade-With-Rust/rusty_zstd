//! GATE 15: the LATENCY question the op count cannot answer.
//!
//! At a 1.00 op ratio the two paths still differ in dependency chain:
//!   avx2 : load32 -> vpcmpeqb -> vpmovmskb (SIMD->GPR) -> test -> branch
//!   words: load8  -> xor -> test -> branch
//! plus AVX2 is reached by a tail call while the word loop is inlined.
//! A tight loop isolates this where whole-encode timing cannot.
fn main(){
    let n = 1_000_000usize;
    // build pairs with a controlled common-prefix length
    for plen in [3usize, 5, 7, 12, 20, 40, 100] {
        let mut a = vec![0u8; 256];
        let mut b = vec![0u8; 256];
        for i in 0..256 { a[i] = (i as u8).wrapping_mul(31); }
        b.copy_from_slice(&a);
        b[plen] = a[plen].wrapping_add(1);   // first difference at `plen`
        let mut best_a = f64::MAX;
        let mut best_w = f64::MAX;
        for _ in 0..7 {
            let t = std::time::Instant::now();
            let mut acc = 0usize;
            for _ in 0..n { acc += rusty_zstd::bench_eq_avx2(&a, &b); }
            let e = t.elapsed().as_secs_f64()*1e9/n as f64;
            if e < best_a { best_a = e; }
            std::hint::black_box(acc);
            let t = std::time::Instant::now();
            let mut acc = 0usize;
            for _ in 0..n { acc += rusty_zstd::bench_eq_words(&a, &b); }
            let e = t.elapsed().as_secs_f64()*1e9/n as f64;
            if e < best_w { best_w = e; }
            std::hint::black_box(acc);
        }
        let verdict = if best_w < best_a*0.97 {"WORDS faster"} else if best_a < best_w*0.97 {"avx2 faster"} else {"tie"};
        println!("prefix {plen:>4}B: avx2 {best_a:>6.2} ns   words {best_w:>6.2} ns   {:>+7.1}%   {verdict}",
            100.0*(best_w-best_a)/best_a);
    }
}
