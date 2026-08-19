//! SENSITIVITY: at what prefix length does the output actually start to change?
//! Without this, a byte-identity pass proves nothing -- it may just mean the
//! payload never reaches deep history at these sizes.
fn main() {
    const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m","text-32m"];
    for &lvl in &[3i32, 19] {
        println!("\n=== L{lvl} — prefix kept vs output changed (vs the FULL 4 MiB prefix) ===");
        let mut hdr = format!("{:<13}", "corpus");
        let fracs: &[(&str, usize)] = &[("win+blk", 0), ("win", 1), ("win/2", 2), ("win/4", 4), ("win/8", 8), ("128K", 0), ("32K", 0)];
        for (n, _) in fracs { hdr += &format!(" {:>8}", n); }
        println!("{hdr}");
        for id in IDS {
            let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            if full.len() < (5 << 20) { continue }
            let pre = &full[..4 << 20];
            let tail = &full[4 << 20..(4 << 20) + (1 << 20)];
            let p = rusty_zstd::compression_params(lvl, Some(tail.len() as u64)).unwrap();
            let window = 1usize << p.window_log.min(31);
            let z_ref = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            let mut row = format!("{:<13}", id);
            for (name, div) in fracs {
                let keep = match *name {
                    "win+blk" => window + rusty_zstd::BLOCKSIZE_MAX as usize,
                    "128K" => 128 << 10,
                    "32K" => 32 << 10,
                    _ => window / div,
                };
                let k = keep.min(pre.len());
                let z = rusty_zstd::compress_using_prefix(tail, &pre[pre.len()-k..], lvl).unwrap();
                let d = z.len() as i64 - z_ref.len() as i64;
                row += &format!(" {:>8}", if z == z_ref { "same".to_string() } else { format!("{d:+}") });
            }
            println!("{row}");
        }
    }
}
