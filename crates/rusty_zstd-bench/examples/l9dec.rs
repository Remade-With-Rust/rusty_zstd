//! WHY DOES L9 DECODE SLOWER THAN L1? The per-sequence work profile, by level.
//!
//! `dsanat` part (a) shows L9 decoding at 3.12 ms/MiB against L1's 2.50 -- 25%
//! SLOWER despite compressing 20% better. That is backwards for a format where
//! better compression means fewer sequences to replay, so the per-sequence work
//! must be shifting somewhere.
//!
//! This holds the decode side still and varies only the LEVEL, reporting the
//! deterministic per-sequence counts and the `copy_match` band split. Every
//! column is a census -- identical on any machine at any load.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example l9dec

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];
const BANDS: [&str; 8] = [
    "off==1 splat",
    "32B (len>16)",
    "16B tier",
    "extend_within",
    "overlap chunk",
    "32B (len<=16)",
    "64B tier",
    "(unused)",
];

fn main() {
    let cap: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 << 20);

    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    (*id, f[..n].to_vec())
                })
        })
        .collect();
    let total: u64 = srcs.iter().map(|(_, s)| s.len() as u64).sum();

    println!("L9 DECODE WORK PROFILE -- counts per level, {} corpora, {:.0} MiB\n",
             srcs.len(), total as f64 / (1 << 20) as f64);
    println!("{:>5} {:>12} {:>11} {:>11} {:>10} {:>11} {:>10}",
             "level", "sequences", "seq/MiB", "matchB/seq", "litB/seq", "csize", "ratio");

    let mut rows = Vec::new();
    for lvl in [1i32, 3, 5, 9, 19] {
        let mut nseq = 0u64;
        let mut csize = 0u64;
        let mut bands = [0u64; 8];
        let mut band_b = [0u64; 8];
        let mut mb = 0u64;
        let mut lb = 0u64;
        for (_, s) in &srcs {
            let z = rusty_zstd::compress(s, lvl).expect("compress");
            csize += z.len() as u64;
            let _ = rusty_zstd::take_dec_bands();
            rusty_zstd::prof_reset();
            let out = rusty_zstd::decompress(&z).expect("decompress");
            assert_eq!(out.len(), s.len());
            let (b, bb) = rusty_zstd::take_dec_bands();
            for i in 0..8 {
                bands[i] += b[i];
                band_b[i] += bb[i];
            }
        }
        // Sequence count is the sum of all band calls: every sequence takes
        // exactly one `copy_match` route.
        nseq = bands.iter().sum();
        mb = band_b.iter().sum();
        lb = total.saturating_sub(mb);
        println!(
            "{:>5} {:>12} {:>11.0} {:>11.1} {:>10.1} {:>11} {:>10.3}",
            lvl,
            nseq,
            nseq as f64 / (total as f64 / (1 << 20) as f64),
            if nseq > 0 { mb as f64 / nseq as f64 } else { 0.0 },
            if nseq > 0 { lb as f64 / nseq as f64 } else { 0.0 },
            csize,
            total as f64 / csize as f64
        );
        rows.push((lvl, bands, band_b, nseq));
    }

    println!("\ncopy_match BAND SPLIT -- % of calls (the route each sequence takes)\n");
    print!("{:>5}", "level");
    for b in BANDS.iter().take(7) {
        print!("{:>15}", b);
    }
    println!();
    for (lvl, bands, _, nseq) in &rows {
        print!("{:>5}", lvl);
        for i in 0..7 {
            print!("{:>14.1}%", if *nseq > 0 { 100.0 * bands[i] as f64 / *nseq as f64 } else { 0.0 });
        }
        println!();
    }

    println!(
        "\nseq/MiB is the DecSeq loop's trip count -- the single biggest term in\n\
         decode. If L9 decodes slower than L1 while running FEWER trips, the\n\
         cost has moved INTO the trip, and the band split says which route."
    );
}
