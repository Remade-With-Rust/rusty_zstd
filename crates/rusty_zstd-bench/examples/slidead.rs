//! Paired A/B of the streaming slide multiplier, in ONE process.
//!
//! Both arms are in this binary and the arm is chosen per Compressor, so there
//! is no rebuild and no second executable between the two measurements.
//!
//! Discipline (codec-measurement): arms ABBA-interleaved so machine drift
//! cancels rather than landing between blocks; a NULL arm (k=2 against itself)
//! establishes what this box can resolve at all; paired win-rate with a z-score
//! rather than a ratio of medians, because on a drifting box the medians move
//! more than the effect. The deterministic counters -- prime inserts, bytes
//! moved, compressed size -- are the primary evidence; this only prices them.
use rusty_zstd::{Compressor, Flush};
use std::time::Instant;

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}

fn run(src: &[u8], lvl: i32, mul: usize, chunk: usize) -> (f64, usize) {
    let mut c = Compressor::new(lvl).expect("compressor");
    c.set_slide_mul(mul);
    let mut out = Vec::with_capacity(src.len() / 2 + (1 << 20));
    let mut buf = vec![0u8; 128 << 10];
    let t = Instant::now();
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
        if st.done || st.output_produced == 0 {
            break;
        }
    }
    (t.elapsed().as_secs_f64(), out.len())
}

fn verdict(name: &str, a: &[f64], b: &[f64]) {
    let n = a.len();
    let wins = a.iter().zip(b).filter(|(x, y)| y < x).count();
    let ties = a
        .iter()
        .zip(b)
        .filter(|(x, y)| (**y - **x).abs() < 1e-9)
        .count();
    let eff = n - ties;
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
    let (ma, mb) = (med(a), med(b));
    let mn = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
    println!(
        "  {name:<22} median {ma:.4}s -> {mb:.4}s ({:.3}x)  min {:.4} -> {:.4} ({:.3}x)  \
{wins}/{eff} z={z:+.2}",
        ma / mb,
        mn(a),
        mn(b),
        mn(a) / mn(b)
    );
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let reps: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(21);
    let ka: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let kb: usize = std::env::args()
        .nth(4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let chunk = 64 << 10;
    for id in ["samba", "webster", "mozilla"] {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        let (_, s2) = run(src, lvl, ka, chunk);
        let (_, s3) = run(src, lvl, kb, chunk);
        println!(
            "\n{id}  {:.1} MiB  compressed k=2 {s2} B, k=3 {s3} B ({:+.4}% size)",
            src.len() as f64 / (1 << 20) as f64,
            (s3 as f64 - s2 as f64) / s2 as f64 * 100.0
        );
        let (mut a, mut b, mut na, mut nb) = (vec![], vec![], vec![], vec![]);
        for r in 0..reps {
            // ABBA: alternate which arm leads, so "the second one runs warmer"
            // cancels instead of accumulating into one arm.
            if r % 2 == 0 {
                a.push(run(src, lvl, ka, chunk).0);
                b.push(run(src, lvl, kb, chunk).0);
                na.push(run(src, lvl, ka, chunk).0);
                nb.push(run(src, lvl, ka, chunk).0);
            } else {
                b.push(run(src, lvl, kb, chunk).0);
                a.push(run(src, lvl, ka, chunk).0);
                nb.push(run(src, lvl, ka, chunk).0);
                na.push(run(src, lvl, ka, chunk).0);
            }
        }
        verdict("NULL (same arm twice)", &na, &nb);
        verdict("A -> B", &a, &b);
    }
    println!(
        "\nRead the NULL arm first: it is what this box can resolve. A k=3 verdict\n\
         inside the null's spread is not a result."
    );
}
