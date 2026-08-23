//! BYTES-problem probe: would pooling `MatchTables` help or HURT?
//!
//! Fresh `vec![0; n]` gets lazily-zeroed pages -- the OS materialises only what
//! is touched. Pooling requires `reset()`, which is `.fill(0)` over the WHOLE
//! table. So the question is what FRACTION of the table the encoder touches: if
//! it is well under 1, pooling does strictly more work.
//!
//! Deterministic: table sizes are a function of (level, input size), and the
//! store count is bounded by the position count. No clock.
fn main() {
    for (lvl, mib) in [(19i32, 8usize), (19, 32), (9, 8), (3, 8), (1, 8)] {
        let n = mib << 20;
        let p = rusty_zstd::compression_params(lvl, Some(n as u64)).unwrap();
        let hlog = p.hash_log.clamp(6, 24);
        let clog = p.chain_log.min(24);
        let strat = format!("{:?}", p.strategy);
        let hsz = 1usize << hlog;
        let csz = 1usize << clog;
        let hbytes = hsz * 4;
        let cbytes = csz * 4;
        println!("L{lvl} {mib} MiB [{strat}]  hash 2^{hlog} = {:.1} MiB   chain 2^{clog} = {:.1} MiB",
            hbytes as f64 / (1<<20) as f64, cbytes as f64 / (1<<20) as f64);
        println!("    positions <= {n} ; hash slots {hsz}  ->  at most {:.1}% of the hash table can ever be touched",
            100.0 * (n as f64 / hsz as f64).min(1.0));
        println!("    reset() would memset {:.1} MiB every reuse",
            (hbytes + cbytes) as f64 / (1<<20) as f64);
    }
}
