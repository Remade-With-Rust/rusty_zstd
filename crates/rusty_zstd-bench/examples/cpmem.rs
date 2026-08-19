//! Does the clamp BIND, and what does it save DETERMINISTICALLY?
//! Bytes allocated per frame is arithmetic; the clock is not.
fn main() {
    println!("{:>5} {:>10} {:>16} {:>16} {:>12} {:>10}", "lvl", "payload", "off h/c log", "on h/c log", "off MiB", "on MiB");
    for &cap in &[256usize<<10, 1<<20, 4<<20, 16<<20] {
        for &lvl in &[1i32, 3, 13, 16, 19, 22] {
            rusty_zstd::set_cparam_clamp_arm(false);
            let a = rusty_zstd::compression_params(lvl, Some(cap as u64)).unwrap();
            rusty_zstd::set_cparam_clamp_arm(true);
            let b = rusty_zstd::compression_params(lvl, Some(cap as u64)).unwrap();
            let bt = |p: &rusty_zstd::CompressionParameters| -> f64 {
                let hash = (1u64 << p.hash_log.min(24)) * 4;
                let uses_chain = (p.strategy as u32) >= 3;
                let chain = if uses_chain { (1u64 << p.chain_log.min(24)) * 4 } else { 0 };
                (hash + chain) as f64 / 1048576.0
            };
            let (ma, mb) = (bt(&a), bt(&b));
            if a.hash_log != b.hash_log || a.chain_log != b.chain_log {
                println!("{:>5} {:>9}K {:>10}/{:<5} {:>10}/{:<5} {:>12.1} {:>10.1}   -{:.0}%",
                    lvl, cap>>10, a.hash_log, a.chain_log, b.hash_log, b.chain_log, ma, mb, (1.0-mb/ma)*100.0);
            } else {
                println!("{:>5} {:>9}K {:>10}/{:<5} {:>10}/{:<5} {:>12.1} {:>10.1}   (no bind)",
                    lvl, cap>>10, a.hash_log, a.chain_log, b.hash_log, b.chain_log, ma, mb);
            }
        }
    }
    rusty_zstd::set_cparam_clamp_arm(true);
}
