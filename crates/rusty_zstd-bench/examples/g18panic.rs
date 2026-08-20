//! GATE 18 step 3: min_match is clamped to 3..=7 in `compression_params`, the
//! DERIVATION point -- but a caller who derives then mutates the struct bypasses
//! it, which is exactly what every sweep in 4.65-4.67 did. Same shape as the
//! hash_log panic: validated in one place, used in another. Probe for failures.
fn main(){
    let src: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let mut bad = 0;
    for mm in [0u32, 1, 2, 3, 7, 8, 9, 16, 64, 255, 1024, 65535, u32::MAX] {
        for lvl in [1i32, 3, 5, 7, 13, 19, 22] {
            let mut p = match rusty_zstd::compression_params(lvl, Some(src.len() as u64)) {
                Ok(p) => p, Err(_) => continue,
            };
            p.min_match = mm;
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rusty_zstd::compress_with_params(&src, p, false)
            }));
            match r {
                Err(_) => { println!("PANIC   min_match={mm:<11} L{lvl}"); bad += 1; }
                Ok(Err(e)) => println!("Err     min_match={mm:<11} L{lvl}: {e:?}"),
                Ok(Ok(z)) => {
                    let d = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        rusty_zstd::decompress(&z)
                    }));
                    match d {
                        Err(_) => { println!("PANIC on decode  min_match={mm:<7} L{lvl}"); bad += 1; }
                        Ok(Ok(v)) if v == src => {}
                        _ => { println!("ROUNDTRIP FAIL   min_match={mm:<7} L{lvl}"); bad += 1; }
                    }
                }
            }
        }
    }
    println!("\nmin_match probe: {bad} failures across 13 values x 7 levels");
}
