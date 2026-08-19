//! Where does FINDING 1 actually bind? window_log = min(level_window, src_log),
//! so widening src_log by the dictionary only matters when the LEVEL's window is
//! not already the smaller of the two.
fn main() {
    println!("{:>5} {:>10} {:>10} {:>12} {:>12} {:>8}", "lvl", "dict KiB", "pay KiB", "wlog payload", "wlog pay+dict", "binds?");
    for &lvl in &[1i32, 3, 5, 9, 13, 16, 19, 22] {
        for &(dk, pk) in &[(4096usize, 1024usize), (112, 4096)] {
            let (dn, pn) = (dk << 10, pk << 10);
            let a = rusty_zstd::compression_params(lvl, Some(pn as u64)).unwrap();
            let b = rusty_zstd::compression_params(lvl, Some((pn + dn) as u64)).unwrap();
            println!("{:>5} {:>10} {:>10} {:>12} {:>12} {:>8}", lvl, dk, pk, a.window_log, b.window_log,
                if a.window_log != b.window_log { "YES" } else { "no" });
        }
    }
}
