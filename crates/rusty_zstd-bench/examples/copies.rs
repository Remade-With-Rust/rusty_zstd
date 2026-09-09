//! COPY CENSUS -- bytes moved per input byte, per site, on the encode path.
//!
//! Deterministic: byte totals are a property of the input and the code path,
//! so this reads the same on any machine at any load.
//!
//! The number to look at is the LAST column: copies per input byte. It has a
//! floor above zero -- an encoder must place literal bytes into its output --
//! so the target is not "0", it is "no byte moved a SECOND time".
use rusty_zstd::copies::{self, COPY_NAMES, N_COPY_SLOTS};

const IDS: &[&str] = &[
    "incomp-32m",
    "text-32m",
    "jsonlog-16m",
    "versions-16m",
    "dickens",
    "samba",
    "webster",
    "x-ray",
    "mozilla",
];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .ok()
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    println!("ENCODE COPY CENSUS (L{lvl}) -- bytes, not clocks\n");
    println!(
        "{:<14}{:>9}{:>14}{:>14}{:>14}{:>10}",
        "corpus", "MiB", "src->lits", "lits->rawsec", "sec->dst", "cp/byte"
    );

    let mut tot = [(0u64, 0u64); N_COPY_SLOTS];
    let mut tsrc = 0u64;
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        let _ = copies::take();
        let z = rusty_zstd::compress(src, lvl).expect("compress");
        let c = copies::take();
        // Round-trip so a copy change can never silently break output.
        let out = rusty_zstd::decompress(&z).expect("decompress");
        assert_eq!(out, src, "{id} roundtrip");
        let _ = copies::take();

        let moved: u64 = c.iter().map(|(b, _)| *b).sum();
        let per = moved as f64 / src.len() as f64;
        println!(
            "{id:<14}{:>9.1}{:>14}{:>14}{:>14}{:>10.3}",
            src.len() as f64 / (1 << 20) as f64,
            c[copies::C_LIT_PUSH].0,
            c[copies::C_LIT_RAW_SECTION].0,
            c[copies::C_SECTION_TO_DST].0,
            per
        );
        tsrc += src.len() as u64;
        for i in 0..N_COPY_SLOTS {
            tot[i].0 += c[i].0;
            tot[i].1 += c[i].1;
        }
    }

    println!(
        "\n{:<24}{:>16}{:>14}{:>12}",
        "site", "bytes", "calls", "B/input"
    );
    let mut moved = 0u64;
    for i in 0..N_COPY_SLOTS {
        let (b, n) = tot[i];
        if b == 0 && n == 0 {
            continue;
        }
        moved += b;
        println!(
            "{:<24}{b:>16}{n:>14}{:>12.4}",
            COPY_NAMES[i],
            b as f64 / tsrc as f64
        );
    }
    println!(
        "\nTOTAL {moved} bytes moved for {tsrc} input bytes = {:.3} copies per input byte",
        moved as f64 / tsrc as f64
    );
    println!(
        "floor is ~1.0 on literal-heavy input (the bytes must reach the output);\n\
         anything above that is a byte moved a second time."
    );
}
