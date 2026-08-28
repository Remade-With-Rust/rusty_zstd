//! DO THE 41.7M BACK-FILL INSERTS EARN THEIR KEEP? A ceiling probe.
//!
//! `find_lazy_impl` re-inserts every position a match covered (defect B1:
//! without it, bytes inside matches are absent from the finder's table and
//! later searches find worse matches). `lazyfill.rs` prices that at 12.21
//! inserts per site and 2.31x the finder's own probe count, and in ROW mode
//! every one is a random write into an 80 MB table.
//!
//! The stride knob already exists. This sweeps it and reports WORK against
//! SIZE, so the trade is a table instead of an argument. Stride 1 is C's
//! behaviour and the shipping default; larger strides thin the fill.
//!
//! Bitstream-CHANGING at stride > 1 -- the size column IS the price.
//! Requires --features profile.
const IDS: &[&str] = &["dickens", "webster", "samba", "xml", "nci", "reymont", "osdb", "mozilla"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| load(id).map(|f| (*id, f[..f.len().min(8 << 20)].to_vec())))
        .collect();
    let rows_on = std::env::args().nth(2).map(|v| v != "0").unwrap_or(true);
    rusty_zstd::set_row_arm(rows_on);
    println!(
        "BACK-FILL STRIDE SWEEP @ L{lvl}, row arm {}, {} corpora\n",
        if rows_on { "ON" } else { "OFF -- CHAIN, the SHIPPING path" },
        srcs.len()
    );
    println!("{:>7}{:>15}{:>14}{:>13}{:>11}{:>14}", "stride", "fill inserts", "vs stride 1", "size", "vs stride 1", "ROW probes");
    let mut base_ins = 0u64;
    let mut base_sz = 0u64;
    for stride in [1usize, 2, 3, 4, 8] {
        rusty_zstd::set_lazy_fill_stride_arm(stride);
        let (mut ins, mut sz, mut probes) = (0u64, 0u64, 0u64);
        for (id, src) in &srcs {
            let _ = rusty_zstd::take_lazy_fill();
            let z = rusty_zstd::compress(src, lvl).unwrap();
            assert_eq!(&rusty_zstd::decompress(&z).unwrap(), src, "{id}: round-trip");
            ins += rusty_zstd::take_lazy_fill().2;
            probes += rusty_zstd::take_row_walk()[0];
            sz += z.len() as u64;
        }
        if stride == 1 {
            base_ins = ins;
            base_sz = sz;
        }
        println!(
            "{:>7}{:>15}{:>13.2}x{:>13}{:>10.4}x{:>14}",
            stride,
            ins,
            ins as f64 / base_ins as f64,
            sz,
            sz as f64 / base_sz as f64,
            probes
        );
    }
    rusty_zstd::set_lazy_fill_stride_arm(1);
    rusty_zstd::set_row_arm(false);
    println!("\nStride 1 is the shipping default. Read work saved against size paid.");
}
