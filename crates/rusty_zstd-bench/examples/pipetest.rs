fn main() {
    let f = std::fs::read("corpora/data/silesia/x-ray").unwrap();
    let s = &f[..f.len().min(6 << 20)];
    for pipe in [true, false] {
        rusty_zstd::set_dfast_pipe_arm(pipe);
        let _ = rusty_zstd::take_long_tag();
        let _ = rusty_zstd::take_long_tag_residual();
        let z = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: 3, checksum: false }).unwrap();
        let (ne, rej, fal) = rusty_zstd::take_long_tag();
        let (sf, sw, sa) = rusty_zstd::take_long_tag_residual();
        println!("pipe={pipe}: bytes {}, load-site ne {ne} rej {rej} false {fal} | consume bytes-fail {sf} win-fail {sw} accepted {sa}", z.len());
    }
}
