//! EQUIVALENCE GATE for the derived `spec_used` (used = made - discarded -
//! leftover): the DFast speculation counters must be BIT-EQUAL to the counted
//! baseline on every corpus, or the derivation identity is wrong. Byte
//! fingerprints cannot catch an error here -- the pipeline gate is
//! byte-identical whichever way it flips -- so the counter itself is the gate.
//! Run under `--features profile` on both arms and diff the output.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    for lvl in [3i32, 4] {
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(6 << 20)];
            let _ = rusty_zstd::take_dfast_spec();
            let _ = rusty_zstd::take_next_long();
            let _ = rusty_zstd::take_nl_band();
            let z = rusty_zstd::compress(s, lvl).unwrap();
            let (made, used) = rusty_zstd::take_dfast_spec();
            let (nlp, nlh, nlg) = rusty_zstd::take_next_long();
            let (bh, bg, bo) = rusty_zstd::take_nl_band();
            println!(
                "L{lvl} {id:<14} made={made:<10} used={used:<10} nl={nlp}/{nlh}/{nlg} band={bh}/{bg}/{bo} zlen={}",
                z.len()
            );
        }
    }
}
