//! `compress_with_params` takes a CompressionParameters struct and applies NO
//! validation -- every clamp lives in `compression_params`, the derivation point.
//! min_match survives out-of-range values by luck (each consumer re-clamps);
//! hash_log did not, and panicked. Probe every field the same way.
fn main(){
    let src: Vec<u8> = (0..2_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let vals = [0u32, 1, 2, 3, 5, 8, 16, 24, 31, 32, 64, 1024, 65535, u32::MAX];
    let fields: &[(&str, fn(&mut rusty_zstd::CompressionParameters, u32))] = &[
        ("window_log",    |p,v| p.window_log = v),
        ("hash_log",      |p,v| p.hash_log = v),
        ("chain_log",     |p,v| p.chain_log = v),
        ("search_log",    |p,v| p.search_log = v),
        ("min_match",     |p,v| p.min_match = v),
        ("target_length", |p,v| p.target_length = v),
    ];
    let mut bad = 0; let mut tested = 0;
    for (name, setf) in fields {
        for &v in &vals {
            for lvl in [1i32, 3, 7, 13, 19, 22] {
                let Ok(mut p) = rusty_zstd::compression_params(lvl, Some(src.len() as u64)) else {continue};
                setf(&mut p, v);
                tested += 1;
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rusty_zstd::compress_with_params(&src, p, false)
                }));
                match r {
                    Err(_) => { println!("PANIC on encode  {name}={v:<11} L{lvl}"); bad += 1; }
                    Ok(Err(_)) => {}
                    Ok(Ok(z)) => {
                        let d = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            rusty_zstd::decompress(&z)
                        }));
                        match d {
                            Err(_) => { println!("PANIC on decode  {name}={v:<11} L{lvl}"); bad += 1; }
                            Ok(Ok(o)) if o == src => {}
                            _ => { println!("ROUNDTRIP FAIL   {name}={v:<11} L{lvl}"); bad += 1; }
                        }
                    }
                }
            }
        }
    }
    println!("\nparameter probe: {bad} failures across {tested} (field, value, level) cells");
}
