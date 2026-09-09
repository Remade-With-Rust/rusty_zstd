//! Does the EXISTING nl dispatch capture the sao loss? Size only -- exact.
//!
//!   cargo run --release -p rusty_zstd-bench --example nldisp
//!
//! `nlhunt` showed the next-long probe wins 468,072 match bytes on `sao` and
//! still costs 15,001 compressed bytes: it commits at `ip + 1`, so it can take
//! a LONGER match at a WORSE OFFSET and lose more in the offset code than it
//! gains in the length code. `next_long_yield` cannot separate that case
//! (x-ray's yield is 100x lower and the probe HELPS it) -- but the encoder
//! already counts the offset trade in `band_worse / band_hits` ->
//! `tables.nl_off_worse`, and `nl_cut_for` already dispatches on it.
//!
//! That dispatch is OFF by default (`NL_DISPATCH_ON != 2` returns the bare 8).
//! This asks the only question that matters: turned on, does it recover sao
//! without costing the fifteen corpora the probe genuinely helps?
use rusty_zstd as rz;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m",
    "versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla",
    "nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = 4 << 20;
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
    }).collect();
    let o = rz::CompressOptions { level: 3, checksum: false };
    let go = |srcs: &Vec<(&str, Vec<u8>)>| -> Vec<usize> {
        srcs.iter().map(|(_, s)| rz::compress_with(s, o).unwrap().len()).collect()
    };
    rz::set_nl_dispatch_arm(false);
    let base = go(&srcs);
    println!("{:<14}{:>12}{:>14}{:>14}", "corpus", "base", "nl_dispatch", "next_long off");
    println!("{}", "-".repeat(56));
    rz::set_nl_dispatch_arm(true);
    let disp = go(&srcs);
    rz::set_nl_dispatch_arm(false);
    rz::set_next_long_arm(false);
    let off = go(&srcs);
    rz::set_next_long_arm(true);
    let (mut nd, mut nf) = (0i64, 0i64);
    for (i, (id, _)) in srcs.iter().enumerate() {
        let d = disp[i] as i64 - base[i] as i64;
        let f = off[i] as i64 - base[i] as i64;
        nd += d; nf += f;
        println!("{:<14}{:>12}{:>+14}{:>+14}", id, base[i], d, f);
    }
    println!("{:<14}{:>12}{:>+14}{:>+14}", "NET", base.iter().sum::<usize>(), nd, nf);
    println!("\nnl_dispatch column: negative = smaller than shipped default.");
}
