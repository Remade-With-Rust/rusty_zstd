//! SIZE BOARD for the Fast and DFast levels: bytegate's eighteen corpora at
//! L1-L4, every frame round-tripped, one row per corpus.
//!
//!   cargo run --release -p rusty_zstd-bench --example l14board -- [cap_bytes]
//!
//! `bytegate` is the identity anchor and covers nine levels but not L4; a
//! bitstream-changing arm on the L1-L4 finders is adjudicated here, where a
//! pass costs seconds and the DFast level bytegate omits is on the board.
//! Run it once per arm (the arms are environment knobs, e.g. `RZSTD_REP2=0`)
//! and diff the rows. Sizes only: a count, identical on any machine.
//!
//! The last line folds every frame's bytes into one FNV-1a 64 per level, so a
//! speed brick that claims byte-identity at L1-L4 compares four numbers.
const IDS: &[&str] = &[
    "zeros-32m",
    "text-32m",
    "incomp-32m",
    "jsonlog-16m",
    "smallmsg-8m",
    "versions-16m",
    "mr",
    "ooffice",
    "osdb",
    "reymont",
    "sao",
    "webster",
    "dickens",
    "mozilla",
    "nci",
    "samba",
    "xml",
    "x-ray",
];
const LEVELS: &[i32] = &[1, 2, 3, 4];

fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    let mut tot = [0usize; 4];
    let mut gold = [0xCBF2_9CE4_8422_2325u64; 4];
    println!("L14BOARD cap={cap}");
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        else {
            continue;
        };
        let s = &f[..f.len().min(cap)];
        print!("{id:<14}");
        for (i, &l) in LEVELS.iter().enumerate() {
            let z = rusty_zstd::compress(s, l).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("decompress")[..], s, "{id} L{l}");
            tot[i] += z.len();
            for &b in &z {
                gold[i] = (gold[i] ^ u64::from(b)).wrapping_mul(0x100_0000_01B3);
            }
            print!("{:>11}", z.len());
        }
        println!();
    }
    println!("{:<14}{:>11}{:>11}{:>11}{:>11}", "TOTAL", tot[0], tot[1], tot[2], tot[3]);
    println!(
        "GOLD L1 {:016X} L2 {:016X} L3 {:016X} L4 {:016X}",
        gold[0], gold[1], gold[2], gold[3]
    );
}
