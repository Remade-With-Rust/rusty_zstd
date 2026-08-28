//! C6 guard: the empty-input seekable path used to be a hand-written special
//! case; it is now one iteration of the main loop. Prove they agree.
fn main() {
    let p = rusty_zstd::compression_params(3, None).expect("params");
    for (label, src) in [
        ("empty", &b""[..]),
        ("one", &b"x"[..]),
        ("multi", &b"hello world hello world"[..]),
    ] {
        for checksum in [false, true] {
            let out = rusty_zstd::compress_seekable(src, p, checksum, 8).expect("compress");
            let t = rusty_zstd::parse_seek_table(&out).expect("parse table");
            let back = rusty_zstd::decompress(&out).expect("decompress");
            println!(
                "{label:6} ck={checksum:<5} bytes={:<4} frames={:<2} usize={:<3} roundtrip={}",
                out.len(),
                t.entries.len(),
                t.uncompressed_size(),
                if back == src { "OK" } else { "MISMATCH" }
            );
            assert_eq!(back, src, "roundtrip {label}");
        }
    }
}
