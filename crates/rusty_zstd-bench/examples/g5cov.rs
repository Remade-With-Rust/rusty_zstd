//! GATE 5 coverage across the WHOLE size range, not just big corpora.
//! An earlier proof used a single 2 MiB file and reported 0 fallbacks; that
//! only ever exercises the large-input parameter rows.
fn main() {
    let big = std::fs::read("corpora/data/silesia/xml").unwrap();
    let sizes = [1usize<<10, 1<<12, 1<<14, 1<<16, 1<<17, 1<<18, 1<<20, 1<<21, 1<<22, big.len()];
    println!("{:>12}{:>10}{:>12}{:>10}{:>12}{:>10}", "size", "L3 spec", "L3 runtime", "L19 spec", "L19 runtime", "L22 rt");
    let (mut d_rt, mut b_rt) = (0u64, 0u64);
    for &n in &sizes {
        let src = &big[..n.min(big.len())];
        let _ = rusty_zstd::take_dfast_calls(); let _ = rusty_zstd::take_bt_calls();
        let _ = rusty_zstd::compress(src, 3).unwrap();
        let (ds, dr) = rusty_zstd::take_dfast_calls();
        let _ = rusty_zstd::take_bt_calls();
        let _ = rusty_zstd::compress(src, 19).unwrap();
        let (bs, br) = rusty_zstd::take_bt_calls();
        let _ = rusty_zstd::compress(src, 22).unwrap();
        let (_, br2) = rusty_zstd::take_bt_calls();
        d_rt += dr; b_rt += br + br2;
        println!("{n:>12}{ds:>10}{dr:>12}{bs:>10}{br:>12}{br2:>10}");
    }
    println!("\nTOTAL runtime fallbacks — DFast: {d_rt}   BT: {b_rt}");
    println!("{}", if d_rt==0 && b_rt==0 { "GATE 5 COMPLETE across the whole size range" } else { "STILL INCOMPLETE" });
}
