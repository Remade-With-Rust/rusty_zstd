//! Where does encode time actually go? Stage shares, not guesses.
use rusty_zstd::ProfStage as S;
const ST: &[(S, &str)] = &[
    (S::EncodeTotal, "EncodeTotal"),
    (S::EncodeBlocks, "  EncodeBlocks"),
    (S::EncodeMatchFind, "    MatchFind"),
    (S::EncodeEntropy, "    Entropy"),
    (S::EncodeHuff, "      Huff"),
    (S::EncodeSeqCode, "      SeqCode"),
    (S::EncodeTableSelect, "      TableSelect"),
    (S::EncodeFseSeq, "      FseSeq"),
    (S::EncodeTables, "  EncodeTables"),
    (S::EncodeChecksum, "  Checksum"),
];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let id = std::env::args().nth(2).unwrap_or_else(|| "dickens".into());
    let f = std::fs::read(format!("corpora/data/silesia/{id}")).expect("corpus");
    let src = &f[..f.len().min(16 << 20)];
    rusty_zstd::prof_reset();
    let z = rusty_zstd::compress(src, lvl).expect("c");
    let tot = rusty_zstd::prof_stage_ns(S::EncodeTotal).max(1) as f64;
    println!("{id} L{lvl}  {:.1} MiB -> {:.1} MiB\n", src.len() as f64/(1<<20) as f64, z.len() as f64/(1<<20) as f64);
    println!("{:<20}{:>12}{:>9}{:>12}", "stage", "ms", "% total", "calls");
    for (s, n) in ST {
        let ns = rusty_zstd::prof_stage_ns(*s);
        let c = rusty_zstd::prof_stage_calls(*s);
        if ns == 0 && c == 0 { continue; }
        println!("{n:<20}{:>12.1}{:>8.1}%{c:>12}", ns as f64/1e6, ns as f64/tot*100.0);
    }
    let mf = rusty_zstd::prof_stage_ns(S::EncodeMatchFind) as f64;
    let en = rusty_zstd::prof_stage_ns(S::EncodeEntropy) as f64;
    println!("\nmatch-find {:.1}%   entropy {:.1}%   everything else {:.1}%",
        mf/tot*100.0, en/tot*100.0, (tot-mf-en)/tot*100.0);
    println!("(profiled build: rdtsc scope pairs inflate absolute ms; read the SHARES)");
}
