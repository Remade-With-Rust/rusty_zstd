//! Is the tag's byte-difference the documented 1-in-2^32 empty-slot collision,
//! or stale-entry position aliasing? Aliasing scales with how far the input
//! spans past 2^24; the collision does not.
fn main() {
    let full = std::fs::read("corpora/data/generated/versions-16m").unwrap();
    println!("versions-16m is {} bytes = 2^{:.2}", full.len(), (full.len() as f64).log2());
    println!("{:>12}{:>10}{:>12}{:>12}{:>10}", "prefix", "2^n", "tag OFF", "tag ON", "delta");
    for shift in [20u32, 21, 22, 23, 24] {
        let n = (1usize << shift).min(full.len());
        let src = &full[..n];
        rusty_zstd::set_tag_arm(false);
        let a = rusty_zstd::compress(src, 1).unwrap().len();
        rusty_zstd::set_tag_arm(true);
        let b = rusty_zstd::compress(src, 1).unwrap().len();
        rusty_zstd::set_tag_arm(false);
        println!("{n:>12}{shift:>10}{a:>12}{b:>12}{:>9.3}%", 100.0*(b as f64-a as f64)/a as f64);
    }
    println!("\nIf the delta grows sharply as the prefix approaches 2^24, the cause is");
    println!("stale-entry aliasing in the 24-bit position residue, NOT the empty-slot collision.");
}
