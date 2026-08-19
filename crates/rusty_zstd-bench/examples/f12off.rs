//! The findings must remain OFF by default: a fresh process must produce the
//! same bytes as explicitly-disabled arms, or the default silently changed.
use rusty_zstd::Dictionary;
fn main() {
    let mut n = 0; let mut diff = 0;
    for id in ["mozilla","webster","nci","samba","osdb","xml","versions-16m"] {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < (5<<20) { continue }
        let (pre, tail) = (&f[..4<<20], &f[4<<20..5<<20]);
        let d = Dictionary::raw(pre.to_vec());
        for lvl in [3i32, 13, 19, 22] {
            let fresh = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
            rusty_zstd::set_prime_bt_tree_arm(false);
            rusty_zstd::set_prefix_window_arm(false);
            let explicit = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
            n += 1;
            if fresh != explicit { diff += 1; println!("  DIFF {id} L{lvl}"); }
        }
    }
    println!("{n} cells: default vs explicitly-off differs on {diff} (must be 0)");
    assert_eq!(diff, 0, "the findings are NOT off by default");
}
