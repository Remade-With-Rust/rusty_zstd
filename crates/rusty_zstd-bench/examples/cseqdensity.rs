//! SEQUENCE-DENSITY PARITY -- our L3 frames vs C zstd's L3 frames.
//!
//! The demo's decode arms each decode THEIR OWN encoder's bitstream, so the
//! decode comparison confounds decoder speed with bitstream shape. Section
//! 13.3 proved our decode excess tracks SEQUENCE COUNT; this asks the question
//! that decides whether the encoder owns part of the decode gap: does C's
//! encoder emit fewer sequences per MiB at the same level?
//!
//! Deterministic: the same `take_dec_bands`/`take_dec_untiered` counters
//! `seqdensity.rs` uses, driven over both provenances. Requires
//! --features profile, and expects `<file>.c<level>.zst` siblings produced by
//! the C reference (the runner script makes them).
const IDS: &[&str] = &["dickens", "mozilla", "samba", "webster", "xml", "nci", "reymont", "osdb"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn density(z: &[u8], want_len: usize) -> (u64, u64) {
    let _ = rusty_zstd::take_dec_bands();
    let _ = rusty_zstd::take_dec_untiered();
    let out = rusty_zstd::decompress(z).expect("decode");
    assert_eq!(out.len(), want_len);
    let (bc, _bb) = rusty_zstd::take_dec_bands();
    let u = rusty_zstd::take_dec_untiered();
    let copies: u64 = bc.iter().sum::<u64>() + u[..8].iter().sum::<u64>();
    (copies, out.len() as u64)
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let dir = std::env::args().nth(2).unwrap_or_else(|| "target/cseq".into());
    println!("{:<10}{:>16}{:>16}{:>10}{:>12}{:>12}", "corpus", "OURS copies", "C copies", "C/ours", "ours c-size", "C c-size");
    let (mut oc, mut cc, mut ob) = (0u64, 0u64, 0u64);
    let (mut osz, mut csz) = (0u64, 0u64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(16 << 20)];
        let ours = rusty_zstd::compress_with(
            src,
            rusty_zstd::CompressOptions { level: lvl, checksum: false },
        )
        .unwrap();
        let (o, b) = density(&ours, src.len());
        let cpath = format!("{dir}/{id}.c{lvl}.zst");
        let Ok(cz) = std::fs::read(&cpath) else {
            println!("{id:<10}  missing {cpath}");
            continue;
        };
        let (c, _) = density(&cz, src.len());
        println!(
            "{:<10}{:>16}{:>16}{:>10.3}{:>12}{:>12}",
            id, o, c, c as f64 / o as f64, ours.len(), cz.len()
        );
        oc += o;
        cc += c;
        ob += b;
        osz += ours.len() as u64;
        csz += cz.len() as u64;
    }
    let mib = ob as f64 / (1u64 << 20) as f64;
    println!(
        "\nTOTAL  ours {} ({:.0}/MiB)   C {} ({:.0}/MiB)   C/ours {:.3}   sizes {} vs {}",
        oc,
        oc as f64 / mib,
        cc,
        cc as f64 / mib,
        cc as f64 / oc as f64,
        osz,
        csz
    );
}
