//! D10 BYTE-IDENTITY GATE: the decode-ahead pipeline against the original loop.
//!
//! The pipeline moves DECODE earlier relative to EXECUTE. That is only sound
//! because `resolve_offset` never reads `out` and the output position is a
//! running sum -- so this gate exists to prove the argument rather than trust
//! it. Every corpus, every level, both arms, decoded output compared BYTE FOR
//! BYTE against each other and against the source.
//!
//! A single differing byte is a corrupt frame. This must print ALL MATCH.
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray","zeros-32m",
    "incomp-32m","versions-16m","text-32m"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main() {
    let levels: Vec<i32> = match std::env::args().nth(1) {
        Some(s) => s.split(',').map(|v| v.parse().unwrap()).collect(),
        None => vec![1, 3, 5, 9, 19],
    };
    let mut cells = 0usize;
    let mut bad = 0usize;
    for lvl in &levels {
        for id in IDS {
            let Some(f) = load(id) else { continue };
            let src = &f[..f.len().min(8 << 20)];
            let z = rusty_zstd::compress(src, *lvl).unwrap();
            rusty_zstd::set_pipeline_arm(false);
            let a = rusty_zstd::decompress(&z).unwrap();
            rusty_zstd::set_pipeline_arm(true);
            let b = rusty_zstd::decompress(&z).unwrap();
            cells += 1;
            if a != b || b != src {
                bad += 1;
                let at = a.iter().zip(b.iter()).position(|(x, y)| x != y);
                println!("MISMATCH L{lvl} {id}: lens {} vs {} vs src {} first-diff {at:?}",
                    a.len(), b.len(), src.len());
            }
        }
    }
    rusty_zstd::set_pipeline_arm(false);
    println!("\nD10 gate: {cells} cells over {} levels", levels.len());
    if bad == 0 {
        println!("ALL MATCH -- pipeline is byte-identical to the original loop.");
    } else {
        println!("{bad} MISMATCHES -- D10 IS NOT BYTE-IDENTICAL. Do not ship.");
        std::process::exit(1);
    }
}
