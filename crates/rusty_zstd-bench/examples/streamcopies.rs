//! STREAMING copy census -- the path the one-shot census never touches.
//!
//! `Compressor` keeps a history window. When it overflows it slides: the
//! retained window is memmoved down by `hist.drain(..drop)`, six match tables
//! are zeroed, and the whole window is re-primed. None of that exists in the
//! one-shot encoder, so a census taken there reads zero for all of it.
//!
//! Deterministic: byte totals depend only on the input and the chunk size.
use rusty_zstd::copies::{self, COPY_NAMES, N_COPY_SLOTS};
use rusty_zstd::{Compressor, Flush};

const IDS: &[&str] = &["dickens", "samba", "webster", "mozilla"];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let chunk: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(64 << 10);
    println!("STREAMING COPY CENSUS (L{lvl}, {chunk} B chunks) -- bytes, not clocks\n");
    println!(
        "{:<12}{:>8}{:>8}{:>16}{:>16}{:>10}",
        "corpus", "MiB", "slides", "hist memmoved", "tables zeroed", "B/input"
    );

    let mut tot = [(0u64, 0u64); N_COPY_SLOTS];
    let mut tsrc = 0u64;
    let mut tcsize = 0u64;
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        let _ = copies::take();
        let _ = rusty_zstd::take_enc_slide();

        let mut c = Compressor::new(lvl).expect("compressor");
        let mut out = Vec::with_capacity(src.len() / 2 + (1 << 20));
        let t0 = std::time::Instant::now();
        let mut buf = vec![0u8; 128 << 10];
        let mut i = 0usize;
        while i < src.len() {
            let end = (i + chunk).min(src.len());
            let mut inp = &src[i..end];
            loop {
                let st = c.stream(inp, &mut buf, Flush::Continue).expect("stream");
                out.extend_from_slice(&buf[..st.output_produced]);
                inp = &inp[st.input_consumed..];
                if inp.is_empty() || (st.input_consumed == 0 && st.output_produced == 0) {
                    break;
                }
            }
            i = end;
        }
        loop {
            let st = c.stream(&[], &mut buf, Flush::End).expect("end");
            out.extend_from_slice(&buf[..st.output_produced]);
            if st.done {
                break;
            }
            if st.output_produced == 0 {
                break;
            }
        }
        let csize = out.len();
        let cc = copies::take();
        let sl = rusty_zstd::take_enc_slide();
        drop(buf);
        {
            let back = rusty_zstd::decompress(&out).expect("decompress");
            assert_eq!(back, src, "{id} streaming roundtrip");
        }
        drop(out);
        let _ = copies::take();

        // Prime inserts are POSITIONS, not bytes. Summing them into a byte
        // total mixes two units that differ by an order of magnitude in cost
        // per unit, which is how a census starts reporting a number that means
        // nothing. Counted and reported, never added.
        let moved: u64 = cc
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != copies::C_PRIME_INSERT)
            .map(|(_, (b, _))| *b)
            .sum();
        println!(
            "{id:<12}{:>8.1}{:>8}{:>16}{:>16}{:>10.3}",
            src.len() as f64 / (1 << 20) as f64,
            sl[0],
            cc[copies::C_HIST_SLIDE].0,
            cc[copies::C_TABLE_CLEAR].0,
            moved as f64 / src.len() as f64
        );
        let secs = t0.elapsed().as_secs_f64();
        let slide_b = (cc[copies::C_HIST_SLIDE].0 + cc[copies::C_TABLE_CLEAR].0) as f64;
        println!(
            "             encode {:.3}s ({:.0} MiB/s); slide traffic {:.1} MB = {:.2}-{:.2} ms at 10-20 GB/s = {:.3}%-{:.3}% of encode",
            secs,
            src.len() as f64 / (1 << 20) as f64 / secs,
            slide_b / 1e6,
            slide_b / 10e9 * 1e3,
            slide_b / 20e9 * 1e3,
            slide_b / 10e9 / secs * 100.0,
            slide_b / 20e9 / secs * 100.0
        );
        println!(
            "             compressed {csize} B ({:.4} ratio)",
            src.len() as f64 / csize as f64
        );
        tcsize += csize as u64;
        tsrc += src.len() as u64;
        for i in 0..N_COPY_SLOTS {
            tot[i].0 += cc[i].0;
            tot[i].1 += cc[i].1;
        }
    }

    println!(
        "\n{:<24}{:>16}{:>12}{:>12}",
        "site", "bytes", "calls", "B/input"
    );
    let mut moved = 0u64;
    for i in 0..N_COPY_SLOTS {
        let (b, n) = tot[i];
        if b == 0 && n == 0 {
            continue;
        }
        if i != copies::C_PRIME_INSERT {
            moved += b;
        }
        println!(
            "{:<24}{b:>16}{n:>12}{:>12.4}",
            COPY_NAMES[i],
            b as f64 / tsrc as f64
        );
    }
    println!(
        "\nTOTAL {moved} bytes moved for {tsrc} input bytes = {:.3} copies per input byte",
        moved as f64 / tsrc as f64
    );
}
