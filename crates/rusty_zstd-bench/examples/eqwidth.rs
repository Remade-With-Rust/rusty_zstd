//! Which `max` bucket does `count_eq_len_ge8` actually get?
//!
//! `max` is `limit - ip` -- the room left in the block, NOT the match length.
//! That distinction decides which arms of `count_eq_len_ge8` deserve to be
//! inlined into `count_match` and which should stay behind a call: an arm that
//! runs on 1% of calls but costs 28 static instructions in the hot function is
//! a bad trade however cheap it is dynamically.
//!
//! `wide_eligible` counts `max >= 64`, i.e. every call that reaches the vector
//! dispatch. Everything else lands in the two small arms.
fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = [
            ("silesia", "dickens"),
            ("silesia", "samba"),
            ("silesia", "webster"),
            ("silesia", "xml"),
            ("generated", "jsonlog-16m"),
        ];
        for lvl in [1i32, 3, 9, 19] {
            let _ = rusty_zstd::take_eqlen_stats();
            for (dir, id) in ids {
                let Ok(f) = std::fs::read(format!("corpora/data/{dir}/{id}")) else {
                    continue;
                };
                let s = &f[..f.len().min(4 << 20)];
                let _ = rusty_zstd::compress_with(
                    s,
                    rusty_zstd::CompressOptions { level: lvl, checksum: false },
                )
                .unwrap();
            }
            let (calls, wide, _h) = rusty_zstd::take_eqlen_stats();
            let pct = if calls == 0 { 0.0 } else { wide as f64 * 100.0 / calls as f64 };
            let small = calls - wide;
            println!(
                "L{lvl:<2} calls {calls:>12}  max>=64 {wide:>12} ({pct:6.3}%)  \
                 max<64 {small:>10} ({:6.3}%)",
                100.0 - pct
            );
        }
    }
}
