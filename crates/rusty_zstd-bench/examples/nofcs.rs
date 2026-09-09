//! Does a frame with NO declared content size still grow its output by doubling?
//!
//! Our streaming compressor omits the Frame_Content_Size unless the caller
//! pledges one, so this is the common shape for streamed frames. Before the
//! extrapolated reserve there was no `reserve` on that path at all.
use rusty_zstd::{copies, Compressor, Flush};

fn no_fcs_frame(src: &[u8], lvl: i32) -> Vec<u8> {
    let mut c = Compressor::new(lvl).expect("c");   // NO set_pledged_src_size
    let mut out = Vec::new();
    let mut buf = vec![0u8; 128 << 10];
    let mut i = 0usize;
    while i < src.len() {
        let end = (i + (64 << 10)).min(src.len());
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

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    println!("{:<12}{:>9}{:>12}{:>16}{:>12}", "corpus", "MiB", "has FCS", "reserved B", "vs output");
    for id in ["dickens", "samba", "webster"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        let z = no_fcs_frame(src, lvl);
        let fcs = rusty_zstd::content_size(&z).expect("hdr");
        let _ = copies::take();
        let out = rusty_zstd::decompress(&z).expect("d");
        let c = copies::take();
        assert_eq!(out, src, "{id} roundtrip");
        println!("{id:<12}{:>9.1}{:>12}{:>16}{:>11.2}x",
            src.len() as f64 / (1<<20) as f64,
            format!("{:?}", fcs.is_some()),
            c[copies::C_DEC_RESERVE].0,
            c[copies::C_DEC_RESERVE].0 as f64 / src.len() as f64);
    }
    println!("\n'has FCS false' + a non-zero reserve = the extrapolation fired on the\npath that previously had none.");
}
