//! Does the clamp bind on FULL Silesia files, and does it move any size?
fn main() {
    const IDS: &[&str] = &["xml","reymont","sao"];
    println!("{:<10} {:>10} {:>12} {:>12} {:>12} {:>12} {:>9}", "corpus", "MiB", "off h/c", "on h/c", "off bytes", "on bytes", "delta");
    let mut moved = 0;
    for &lvl in &[19i32] {
        println!("--- L{lvl}");
        for id in IDS {
            let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
            rusty_zstd::set_cparam_clamp_arm(false);
            let pa = rusty_zstd::compression_params(lvl, Some(full.len() as u64)).unwrap();
            let a = rusty_zstd::compress(&full, lvl).unwrap().len();
            rusty_zstd::set_cparam_clamp_arm(true);
            let pb = rusty_zstd::compression_params(lvl, Some(full.len() as u64)).unwrap();
            let b = rusty_zstd::compress(&full, lvl).unwrap().len();
            if a != b { moved += 1 }
            println!("{:<10} {:>10.1} {:>9}/{:<2} {:>9}/{:<2} {:>12} {:>12} {:>+9}",
                id, full.len() as f64/1048576.0, pa.hash_log, pa.chain_log, pb.hash_log, pb.chain_log, a, b, b as i64 - a as i64);
        }
    }
    println!("\n  cells whose size moved: {moved}");
}
