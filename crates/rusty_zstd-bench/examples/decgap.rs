//! One-shot vs streaming decode of the SAME bytes, measured admissibly.
//!
//! The first cut of this compared two sequential best-of-N blocks and produced
//! ratios from 0.49x to 1.20x -- including "streaming is FASTER than one-shot",
//! which is not a thing. The one-shot arm itself moved 29% between runs, i.e.
//! the denominator drifted further than the effect. codec-measurement: never
//! headline a ratio whose denominator moves more than your improvement.
//!
//! So: both arms in ONE process, ABBA-interleaved so drift cancels instead of
//! landing on one arm, paired win-rate with a z-score, and a NULL arm
//! (one-shot against itself) to establish what this box can resolve at all.
use rusty_zstd::Decompressor;

fn one_shot(z: &[u8], out: &mut Vec<u8>) -> f64 {
    let t = std::time::Instant::now();
    out.clear();
    rusty_zstd::decompress_into(out, z).unwrap();
    t.elapsed().as_secs_f64()
}

fn streaming(z: &[u8], buf: &mut [u8], chunk: usize) -> (f64, usize) {
    let t = std::time::Instant::now();
    let mut d = Decompressor::new();
    let (mut n, mut i) = (0usize, 0usize);
    while i < z.len() {
        let end = (i + chunk).min(z.len());
        let mut inp = &z[i..end];
        // DRAIN FULLY before feeding more. The decoder consumes all input it
        // is handed but emits only what fits `buf`, so feeding a chunk and
        // reading ONCE under-drains: at a 3:1 ratio a 64 KiB chunk yields
        // ~192 KiB, `decoded` accumulates, and the harness measures a
        // backlog no real consumer would build. Loop until it stops emitting.
        loop {
            let st = d.stream(inp, buf, false).expect("s");
            n += st.output_produced;
            inp = &inp[st.input_consumed..];
            if st.input_consumed == 0 && st.output_produced == 0 {
                break;
            }
        }
        i = end;
    }
    loop {
        let st = d.stream(&[], buf, true).expect("d");
        n += st.output_produced;
        if st.output_produced == 0 {
            break;
        }
    }
    (t.elapsed().as_secs_f64(), n)
}

fn verdict(name: &str, a: &[f64], b: &[f64]) {
    let wins = a.iter().zip(b).filter(|(x, y)| y < x).count();
    let ties = a
        .iter()
        .zip(b)
        .filter(|(x, y)| (**y - **x).abs() < 1e-9)
        .count();
    let eff = a.len() - ties;
    let z = if eff == 0 {
        0.0
    } else {
        (wins as f64 - eff as f64 / 2.0) / (0.5 * (eff as f64).sqrt())
    };
    let med = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(|x, y| x.partial_cmp(y).unwrap());
        s[s.len() / 2]
    };
    let mn = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
    println!("  {name:<26} median {:.4}->{:.4}s ({:.3}x)  min {:.4}->{:.4} ({:.3}x)  {wins}/{eff} z={z:+.2}",
        med(a), med(b), med(a)/med(b), mn(a), mn(b), mn(a)/mn(b));
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let reps: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);
    let obits: u32 = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(17);
    let cbits: u32 = std::env::args()
        .nth(4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);
    let chunk = 1usize << cbits;
    for id in ["samba", "webster", "mozilla"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
            continue;
        };
        let src = &f[..f.len().min(32 << 20)];
        let ck = std::env::var("RZSTD_BENCH_NO_CK").is_err();
        let z = rusty_zstd::compress_with(
            src,
            rusty_zstd::CompressOptions {
                level: lvl,
                checksum: ck,
            },
        )
        .expect("compress");
        let mut out = Vec::with_capacity(src.len());
        let mut buf = vec![0u8; 1usize << obits];
        let (_, n) = streaming(&z, &mut buf, chunk);
        assert_eq!(n, src.len());
        println!("\n{id}  {:.1} MiB", src.len() as f64 / (1 << 20) as f64);
        let (mut a, mut b, mut na, mut nb) = (vec![], vec![], vec![], vec![]);
        for r in 0..reps {
            if r % 2 == 0 {
                a.push(one_shot(&z, &mut out));
                b.push(streaming(&z, &mut buf, chunk).0);
                na.push(one_shot(&z, &mut out));
                nb.push(one_shot(&z, &mut out));
            } else {
                b.push(streaming(&z, &mut buf, chunk).0);
                a.push(one_shot(&z, &mut out));
                nb.push(one_shot(&z, &mut out));
                na.push(one_shot(&z, &mut out));
            }
        }
        verdict("NULL (one-shot twice)", &na, &nb);
        verdict("one-shot -> streaming", &a, &b);
    }
    println!(
        "\nRead the NULL first. 'one-shot -> streaming' RATIO BELOW 1.0 means streaming is SLOWER."
    );
}
