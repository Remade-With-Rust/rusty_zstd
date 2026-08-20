//! Regenerate section 3 of m7-anatomy.md: share of encode / share of decode.
//! Leaf stages only -- EncodeEntropy, EncodeBlocks and DecodeBlocks are PARENT
//! scopes and ranking them against their own children is meaningless.
use rusty_zstd::ProfStage as S;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    let mut rows: Vec<(String, [f64; 7])> = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::prof_reset();
        let z = rusty_zstd::compress(s, lvl).unwrap();
        let d = rusty_zstd::decompress(&z).unwrap();
        assert!(d == s, "{id}: round-trip");
        let ns = |st: S| rusty_zstd::prof_stage_ns(st) as f64;
        let et = ns(S::EncodeTotal).max(1.0);
        let dt = ns(S::DecodeTotal).max(1.0);
        rows.push(((*id).to_string(), [
            ns(S::EncodeMatchFind) / et * 100.0,
            ns(S::EncodeHuff) / et * 100.0,
            ns(S::EncodeFseSeq) / et * 100.0,
            ns(S::EncodeSeqCode) / et * 100.0,
            ns(S::DecodeLiterals) / dt * 100.0,
            ns(S::DecodeSeq) / dt * 100.0,
            ns(S::DecodeChecksum) / dt * 100.0,
        ]));
    }
    rows.sort_by(|a, b| b.1[0].partial_cmp(&a.1[0]).unwrap());
    println!("| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |    DecCk |");
    println!("| ------------ | --------: | -------: | -----: | ------: | -------: | -------: | -------: |");
    let (mut mf, mut hf, mut dl, mut ds, mut dc) = (0, 0, 0, 0, 0);
    for (id, v) in &rows {
        let enc_lead = if v[0] >= v[1] { 0 } else { 1 };
        let dec_lead = if v[4] >= v[5] && v[4] >= v[6] { 4 } else if v[5] >= v[6] { 5 } else { 6 };
        if enc_lead == 0 { mf += 1 } else { hf += 1 }
        match dec_lead { 4 => dl += 1, 5 => ds += 1, _ => dc += 1 }
        let cell = |i: usize, lead: usize| if i == lead { format!("**{:.1}**", v[i]) } else { format!("{:.1}", v[i]) };
        println!("| {:<12} | {:>9} | {:>8} | {:>6} | {:>7} | {:>8} | {:>8} | {:>8} |",
            id, cell(0, enc_lead), cell(1, enc_lead), format!("{:.1}", v[2]), format!("{:.1}", v[3]),
            cell(4, dec_lead), cell(5, dec_lead), cell(6, dec_lead));
    }
    println!("\nENCODE leader: MatchFind {mf}/18, Huff {hf}/18");
    println!("DECODE leader: DecLits {dl}/18, DecSeq {ds}/18, DecCk {dc}/18");
}
