//! Enumerate the REACHABLE (hash_log, chain_log) pairs for the Bt strategies,
//! the way the original dead-copy census did: every bt clevel x every input
//! size across the range x the streaming case (size unknown).
fn main() {
    let mut set: Vec<(u32, u32)> = Vec::new();
    let mut push = |p: rusty_zstd::CompressionParameters| {
        let pair = (p.hash_log.min(24), p.chain_log.min(24));
        if !set.contains(&pair) { set.push(pair); }
    };
    for lvl in 13i32..=22 {
        // streaming: size unknown
        push(rusty_zstd::compression_params(lvl, None).unwrap());
        let mut n: u64 = 1024;
        while n <= (256 << 20) {
            push(rusty_zstd::compression_params(lvl, Some(n)).unwrap());
            n += (n / 4).max(1024);
        }
    }
    set.sort();
    println!("{} reachable pairs", set.len());
    let mut line = String::from("            ");
    for (i, (h, c)) in set.iter().enumerate() {
        line.push_str(&format!("({h}, {c}) "));
        if (i + 1) % 7 == 0 { println!("{}", line.trim_end()); line = String::from("            "); }
    }
    if !line.trim().is_empty() { println!("{}", line.trim_end()); }
}
