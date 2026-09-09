//! Streaming round-trip CONTENT gate for the compaction-frequency changes.
//!
//! `deccopies` asserts only the decoded LENGTH. These changes alter when
//! buffers are reclaimed, which cannot change output -- but "cannot" is a claim
//! to test. This compares every byte, across chunk sizes chosen to straddle the
//! compaction triggers, and both streaming and one-shot decode of the same
//! frame must agree.
use rusty_zstd::{Compressor, Decompressor, Flush};

fn stream_compress(src: &[u8], lvl: i32, chunk: usize) -> Vec<u8> {
    let mut c = Compressor::new(lvl).expect("c");
    let mut out = Vec::new();
    let mut buf = vec![0u8; 128 << 10];
    let mut i = 0;
    while i < src.len() {
        let end = (i + chunk).min(src.len());
        let mut inp = &src[i..end];
        loop {
            let st = c.stream(inp, &mut buf, Flush::Continue).expect("s");
            out.extend_from_slice(&buf[..st.output_produced]);
            inp = &inp[st.input_consumed..];
            if inp.is_empty() { break; }
        }
        i = end;
    }
    loop {
        let st = c.stream(&[], &mut buf, Flush::End).expect("e");
        out.extend_from_slice(&buf[..st.output_produced]);
        if st.done { break; }
    }
    out
}

fn stream_decompress(z: &[u8], chunk: usize, obuf: usize) -> Vec<u8> {
    let mut d = Decompressor::new();
    let mut buf = vec![0u8; obuf];
    let mut out = Vec::new();
    let mut i = 0;
    while i < z.len() {
        let end = (i + chunk).min(z.len());
        let mut inp = &z[i..end];
        loop {
            let st = d.stream(inp, &mut buf, false).expect("d");
            out.extend_from_slice(&buf[..st.output_produced]);
            inp = &inp[st.input_consumed..];
            if st.input_consumed == 0 && st.output_produced == 0 { break; }
        }
        i = end;
    }
    loop {
        let st = d.stream(&[], &mut buf, true).expect("f");
        out.extend_from_slice(&buf[..st.output_produced]);
        if st.output_produced == 0 { break; }
    }
    out
}

fn main() {
    let mut checks = 0usize;
    for id in ["dickens", "samba", "webster"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
        let src = &f[..f.len().min(12 << 20)];
        for lvl in [1, 3, 9] {
            let z_one = rusty_zstd::compress(src, lvl).expect("one-shot");
            // one-shot frame, streamed out at several chunk/buffer geometries
            for (ic, ob) in [(1usize << 12, 1usize << 12), (64 << 10, 128 << 10), (1 << 20, 1 << 16)] {
                let got = stream_decompress(&z_one, ic, ob);
                assert_eq!(got.len(), src.len(), "{id} L{lvl} len ic={ic} ob={ob}");
                assert!(got == src, "{id} L{lvl} CONTENT ic={ic} ob={ob}");
                checks += 1;
            }
            // streamed frame, both decoders must agree with the source
            for ic in [16usize << 10, 256 << 10] {
                let z = stream_compress(src, lvl, ic);
                assert!(rusty_zstd::decompress(&z).expect("os") == src, "{id} L{lvl} one-shot dec");
                assert!(stream_decompress(&z, 64 << 10, 128 << 10) == src, "{id} L{lvl} stream dec");
                checks += 2;
            }
        }
    }
    println!("PASS: {checks} streaming round-trips byte-exact across corpora x levels x geometries");
}
