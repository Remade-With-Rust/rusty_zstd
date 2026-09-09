//! Are the chain-link TAGS byte-identical when off? The lazy ladder pays a
//! second multiply per inserted byte and a tag decode per link to reject ~10%
//! of candidates that the first-word compare would reject anyway. If the
//! walk's budget counts links the same way with tags off, the output cannot
//! move -- this checks it, per level, on the bytegate corpora.
fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h ^= u64::from(x);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}
const IDS: &[&str] = &["dickens", "mozilla", "webster", "xml", "samba"];
fn board(levels: &[i32]) -> Vec<(i32, u64, usize)> {
    let mut out = Vec::new();
    for &lvl in levels {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut total = 0usize;
        for id in IDS {
            let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
                continue;
            };
            let src = &full[..full.len().min(16 << 20)];
            let z = rusty_zstd::compress(src, lvl).unwrap();
            fnv(&mut h, &z);
            total += z.len();
        }
        out.push((lvl, h, total));
    }
    out
}
fn main() {
    let levels = [5i32, 7, 9, 12];
    let on = board(&levels); // the default, measured FIRST (armone's rule)
    rusty_zstd::set_chain_tag_arm(false);
    let off = board(&levels);
    println!(
        "{:>3} {:>18} {:>18} {:>11} {:>11}  verdict",
        "L", "tags ON", "tags OFF", "bytes ON", "bytes OFF"
    );
    for ((l, h1, n1), (_, h2, n2)) in on.iter().zip(off.iter()) {
        println!(
            "{:>3} {:>018X} {:>018X} {:>11} {:>11}  {}",
            l,
            h1,
            h2,
            n1,
            n2,
            if h1 == h2 { "IDENTICAL" } else { "DIFFERENT" }
        );
    }
}
