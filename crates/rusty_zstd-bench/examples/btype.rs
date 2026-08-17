fn main() {
    for id in ["zeros-32m","text-32m","incomp-32m","x-ray","sao","xml","versions-16m"] {
        let src = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
        let src = &src[..src.len().min(8*1024*1024)];
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::compress(src, 19).unwrap();
        let c = rusty_zstd::prof_encode_counts();
        let tot = c.rle_blocks + c.raw_blocks + c.comp_blocks;
        let verdict = if c.rle_blocks*100 >= tot*90 { "RLE - finder NEVER RUNS" }
            else if c.raw_blocks*100 >= tot*90 { "RAW - finder output DISCARDED (Gate 16)" }
            else { "COMPRESSED - finder output USED (Gate 1)" };
        println!("{id:<14} rle={:<5} raw={:<5} comp={:<5}  {verdict}", c.rle_blocks, c.raw_blocks, c.comp_blocks);
    }
}
