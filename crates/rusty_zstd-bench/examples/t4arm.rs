//! Does `set_matchcopy_arm` actually control the match-copy fast paths?
//!
//! With the arm OFF, every call must fall through to `extend_from_within` / the
//! overlap loop -- the 16- and 32-byte tiers must show ZERO. With it ON they
//! must carry the traffic. And both arms must decode to the same bytes.
const NAMES: [&str; 6] = ["offset-1", "32B tier", "16B tier", "extend_from_within", "overlap loop", "32B len<=16"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = 4 << 20;
    for arm in [true, false] {
        let mut tc = [0u64; 6];
        let mut ok = 0;
        for id in ["dickens", "samba", "mozilla", "x-ray"] {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            let z = rusty_zstd::compress(s, lvl).unwrap();
            rusty_zstd::set_matchcopy_arm(arm);
            let _ = rusty_zstd::take_dec_bands();
            let d = rusty_zstd::decompress(&z).unwrap();
            assert!(d == s, "{id}: arm={arm} DECODE MISMATCH");
            ok += 1;
            let (c, _) = rusty_zstd::take_dec_bands();
            for i in 0..6 { tc[i] += c[i]; }
        }
        println!("arm={arm:<5} round-trip ok on {ok} corpora");
        for i in 0..6 { println!("    {:<20} {}", NAMES[i], tc[i]); }
    }
    println!("\n  EXPECT: arm=false must show 0 in the 32B and 16B tiers.");
}
