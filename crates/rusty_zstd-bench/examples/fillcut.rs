//! DFAST FILL-COUNT BOARD -- what each of the four per-match table writes buys.
//!
//! Section 7's corrected census makes the fill the dominant per-position work at
//! the DEFAULT level: 0.251 fills per input byte against 0.067 probes, i.e.
//! **3.77 table writes per sequence** across two tables at two positions
//! (`match_ip + 2` and `match_end - 2`).
//!
//! Three micro-optimisations of the fill's INSTRUCTIONS measured zero (LLVM had
//! already CSE'd the duplicate loads and hoisted the field reads), so the lever
//! is the COUNT, not the cost of each. This boards it directly: for each
//! setting of `dfast_fill_ends`, the fills actually performed against the
//! compressed size they buy.
//!
//! Both columns are deterministic -- the fill count is a census and the size is
//! the bitstream -- so this board reads the same on any machine at any load.
//! Same shape as the `row_fill_stride` board that took the row finder to
//! stride 2.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example fillcut [level]

const IDS: &[&str] = &[
    "x-ray", "osdb", "jsonlog-16m", "smallmsg-8m", "ooffice", "sao", "dickens", "samba", "nci",
    "webster", "mozilla", "mr",
];

/// ARM MAPPING, and it is off by one in a way that scrambled this board's first
/// run. `set_dfast_fill_n_arm(n)` STORES `n + 1`, and `dfast_fill_ends` then
/// matches the STORED value: 1 -> (false,false), 2 -> (true,false),
/// 4 -> (false,true), anything else -> (true,true). So the argument to pass is
/// one less than the stored arm, and `2` lands on the `_` arm, not on a
/// disabled one.
const ARMS: &[(u8, &str)] = &[
    (2, "both ends (was default)"),
    (1, "start only  (a) DEFAULT"),
    (3, "end only    (b)"),
    (0, "neither     (none)"),
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args()
        .nth(2)
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

    println!("DFAST FILL-COUNT BOARD @ L{lvl} ({} MiB cap, {} corpora)\n", cap >> 20, srcs.len());
    println!(
        "{:<22} {:>14} {:>8} {:>14} {:>9}",
        "fill arm", "fills", "vs base", "bytes", "size"
    );

    let mut base_fills = 0u64;
    let mut base_bytes = 0u64;
    for (i, (arm, label)) in ARMS.iter().enumerate() {
        rusty_zstd::set_dfast_fill_n_arm(*arm);
        let (mut fills, mut bytes) = (0u64, 0u64);
        for (id, s) in &srcs {
            rusty_zstd::prof_reset();
            let z = rusty_zstd::compress(s, lvl).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..], "{id}");
            fills += rusty_zstd::prof_encode_counts().hash_fills;
            bytes += z.len() as u64;
        }
        if i == 0 {
            base_fills = fills;
            base_bytes = bytes;
        }
        println!(
            "{:<22} {:>14} {:>7.2}x {:>14} {:>8.3}%",
            label,
            fills,
            if base_fills > 0 { fills as f64 / base_fills as f64 } else { 0.0 },
            bytes,
            if base_bytes > 0 {
                100.0 * (bytes as f64 - base_bytes as f64) / base_bytes as f64
            } else {
                0.0
            }
        );
    }
    rusty_zstd::set_dfast_fill_n_arm(0);

    println!(
        "\nfills is a CENSUS and bytes is the BITSTREAM -- both deterministic.\n\
         A row that cuts fills hard for a small size cost is the trade the row\n\
         finder already took at stride 2."
    );
}
