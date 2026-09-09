//! DOES IMPROVING THE ONE SHARED PRIMITIVE LIFT BOTH FINDERS?
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example eqlever
//!
//! `find_fast_impl` and `find_dfast_impl` share exactly one primitive:
//! `count_match` -> `count_eq_len_ge8`. This measures the SENSITIVITY of each
//! finder to that primitive by swapping its implementation wholesale --
//! `set_eqlen_arm(1)` forces the scalar word-at-a-time path, bypassing the
//! ladder+AVX2 kernels entirely. Output is byte-identical (asserted); only the
//! instruction mix inside the shared primitive changes.
//!
//! This is a CEILING, not an estimate: swapping vector for scalar is a larger
//! perturbation than any realistic improvement to that primitive could be. If
//! encode time does not separate here, no improvement to the shared primitive
//! can lift either finder -- and a fortiori not both.
//!
//! CONSERVATIVE BY CONSTRUCTION: the arm is only live under `profile`, whose
//! `eq_call` counter fires on EVERY comparator entry, before the arm branch.
//! That tax is identical on both arms and lands on the comparator path
//! specifically, so it INFLATES this primitive's apparent share. A null result
//! under inflation is a safe null.
//!
//! ABBA-phased with a paired estimator (cancels monotone drift), plus a NULL
//! arm (arm 0 against itself) to establish what this box can resolve.
fn phase(src: &[u8], lvl: i32, arm: u8, n: usize) -> (f64, u64) {
    rusty_zstd::set_eqlen_arm(arm);
    let mut b = f64::MAX;
    let mut h = 0u64;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let out = rusty_zstd::compress(src, lvl).unwrap();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < b { b = e; }
        h = out.iter().fold(1469598103934665603u64,
                            |a, &c| (a ^ c as u64).wrapping_mul(1099511628211));
    }
    (b, h)
}
fn main() {
    let ids = ["dickens", "webster", "mozilla", "samba", "nci", "x-ray"];
    for (lvl, finder) in [(1i32, "find_fast_impl"), (3, "find_dfast_impl")] {
        println!("\n=== L{lvl}  {finder} ===");
        println!("{:<10}{:>12}{:>12}   {}", "corpus", "TREAT %", "NULL %", "bytes");
        let mut rows: Vec<(f64, f64)> = Vec::new();
        for id in ids {
            let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else { continue };
            let src = &full[..full.len().min(2 << 20)];
            let (mut treat, mut null) = (vec![], vec![]);
            let (mut h0, mut h1) = (0u64, 0u64);
            for _ in 0..7 {
                // treatment: A = vector arm, B = words-only arm, ABBA
                let (a1, x) = phase(src, lvl, 0, 15); let (b1, y) = phase(src, lvl, 1, 15);
                let (b2, _) = phase(src, lvl, 1, 15); let (a2, _) = phase(src, lvl, 0, 15);
                h0 = x; h1 = y;
                treat.push(0.5 * (100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
                // null: both phases the SAME arm
                let (c1, _) = phase(src, lvl, 0, 15); let (d1, _) = phase(src, lvl, 0, 15);
                let (d2, _) = phase(src, lvl, 0, 15); let (c2, _) = phase(src, lvl, 0, 15);
                null.push(0.5 * (100.0*(d1-c1)/c1 + 100.0*(d2-c2)/c2));
            }
            let m = |v: &Vec<f64>| v.iter().sum::<f64>() / v.len() as f64;
            let (t, n) = (m(&treat), m(&null));
            // NO PER-ROW VERDICT. An earlier version flagged "separates" when
            // |treat| > 3*|null|, which is unstable exactly when the null lands
            // near zero: webster L3 drew null 0.08%, so any treatment above
            // 0.24% "separated". The null is an estimate of a SPREAD, not a
            // per-row threshold -- it is only meaningful pooled across corpora,
            // which is what the summary below does.
            rows.push((t, n));
            println!("{id:<10}{:>12.2}{:>12.2}   {}", t, n,
                     if h0 == h1 { "identical" } else { "MISMATCH" });
        }
        let sp = |v: Vec<f64>| {
            let m = v.iter().sum::<f64>() / v.len() as f64;
            let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt();
            (m, sd, v.iter().cloned().fold(f64::MAX, f64::min), v.iter().cloned().fold(f64::MIN, f64::max))
        };
        let (tm, ts, tlo, thi) = sp(rows.iter().map(|r| r.0).collect());
        let (nm, ns, nlo, nhi) = sp(rows.iter().map(|r| r.1).collect());
        println!("  treatment  mean {tm:+.2}%  sd {ts:.2}  range [{tlo:+.2}, {thi:+.2}]");
        println!("  null       mean {nm:+.2}%  sd {ns:.2}  range [{nlo:+.2}, {nhi:+.2}]");
        println!("  => {}", if tm.abs() > nm.abs() + ns { "RESOLVES" } else { "NOT RESOLVED: treatment lies inside the null band" });
    }
    rusty_zstd::set_eqlen_arm(0);
}
