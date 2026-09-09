//! DECODER streaming compaction census + ceiling.
//!
//! The encoder's window slide bundles three costs (memmove, six table clears,
//! a full re-prime) and the re-prime dominated. The decoder's compaction is
//! the SAME shape of trigger with only ONE of those costs: a memmove. So the
//! bundle argument does not transfer, and the question becomes purely how big
//! that memmove is against how fast decode runs -- and decode runs an order of
//! magnitude faster than encode, so a fixed byte cost is a much larger share.
//!
//! That is the whole reason to measure instead of assuming the encoder's
//! answer carries over.
use rusty_zstd::Decompressor;

const IDS: &[&str] = &["dickens", "samba", "webster", "mozilla"];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}

fn decode_streaming(z: &[u8], chunk: usize) -> (f64, usize) {
    let mut d = Decompressor::new();
    let mut buf = vec![0u8; 128 << 10];
    let mut n = 0usize;
    let t = std::time::Instant::now();
    let mut i = 0usize;
    while i < z.len() {
        let end = (i + chunk).min(z.len());
        let mut inp = &z[i..end];
        loop {
            let st = d.stream(inp, &mut buf, false).expect("stream");
            n += st.output_produced;
            inp = &inp[st.input_consumed..];
            if inp.is_empty() || (st.input_consumed == 0 && st.output_produced == 0) {
                break;
            }
        }
        i = end;
    }
    loop {
        let st = d.stream(&[], &mut buf, true).expect("drain");
        n += st.output_produced;
        if st.output_produced == 0 {
            break;
        }
    }
    (t.elapsed().as_secs_f64(), n)
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
    println!("DECODER STREAMING COMPACTION CENSUS (L{lvl}, {chunk} B chunks)\n");
    println!(
        "{:<12}{:>8}{:>10}{:>16}{:>10}{:>10}{:>22}",
        "corpus", "MiB", "compacts", "bytes memmoved", "B/out", "MiB/s", "memmove as % decode"
    );
    let (mut tb, mut tn, mut tsec) = (0u64, 0u64, 0.0f64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        let z = rusty_zstd::compress(src, lvl).expect("compress");
        let _ = rusty_zstd::take_dec_compact();
        let _ = rusty_zstd::copies::take();
        let (secs, n) = decode_streaming(&z, chunk);
        let c = rusty_zstd::take_dec_compact();
        let cp = rusty_zstd::copies::take();
        assert_eq!(n, src.len(), "{id} decoded length");
        let mbps = src.len() as f64 / (1 << 20) as f64 / secs;
        // At 10-20 GB/s the memmove costs this many ms; against the measured
        // decode wall that is the share a perfect fix could remove.
        let lo = c[1] as f64 / 20e9 / secs * 100.0;
        let hi = c[1] as f64 / 10e9 / secs * 100.0;
        println!(
            "{id:<12}{:>8.1}{:>10}{:>16}{:>10.3}{:>10.0}{:>17.2}-{:.2}%",
            src.len() as f64 / (1 << 20) as f64,
            c[0],
            c[1],
            c[1] as f64 / src.len() as f64,
            mbps,
            lo,
            hi
        );
        println!(
            "             copies/out: in_acc {:.3} + out {:.3} + compact {:.3} + in_compact {:.3} = {:.3}",
            cp[rusty_zstd::copies::C_DEC_IN_ACC].0 as f64 / src.len() as f64,
            cp[rusty_zstd::copies::C_DEC_OUT].0 as f64 / src.len() as f64,
            cp[rusty_zstd::copies::C_DEC_COMPACT].0 as f64 / src.len() as f64,
            cp[rusty_zstd::copies::C_DEC_IN_COMPACT].0 as f64 / src.len() as f64,
            (cp[rusty_zstd::copies::C_DEC_IN_COMPACT].0
                + cp[rusty_zstd::copies::C_DEC_IN_ACC].0
                + cp[rusty_zstd::copies::C_DEC_OUT].0
                + cp[rusty_zstd::copies::C_DEC_COMPACT].0) as f64
                / src.len() as f64
        );
        tb += c[1];
        tn += src.len() as u64;
        tsec += secs;
    }
    println!(
        "\nTOTAL {tb} bytes memmoved for {tn} decoded = {:.3} B/output byte",
        tb as f64 / tn as f64
    );
    println!(
        "ceiling: {:.2}%-{:.2}% of streaming decode ({:.3}s total)",
        tb as f64 / 20e9 / tsec * 100.0,
        tb as f64 / 10e9 / tsec * 100.0,
        tsec
    );
    println!(
        "\nUnlike the encoder's slide, the decoder's compaction is a memmove ALONE --\n\
         no table clear, no re-prime -- so there is no hidden bundled term here.\n\
         That makes this ceiling the whole prize, not a lower bound on it."
    );
}
