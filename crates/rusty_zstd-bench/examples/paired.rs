//! Gate 4/5 re-decided with the PAIRED estimator (null-arm error ~0.1%).
//! Usage: paired <level> <arm: bt|dfast|fast>
fn phase(src: &[u8], lvl: i32, arm: &str, on: bool, n: usize) -> f64 {
    match arm { "bt" => rusty_zstd::set_bt_spec_arm(on),
                "dfast" => rusty_zstd::set_dfast_spec_arm(on),
                _ => rusty_zstd::set_fast_spec_arm(on) }
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let _ = rusty_zstd::compress(src, lvl).unwrap();
        let e = t.elapsed().as_secs_f64()*1000.0;
        if e < b { b = e; }
    }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    let arm = std::env::args().nth(2).unwrap_or_else(|| "bt".into());
    let cap = if lvl >= 16 { 1 } else { 4 } * 1024 * 1024;
    // targeted subset: the corpora the withdrawn L19 test named as "stable
    // generic", plus two controls. Full board is too slow at bt levels.
    let ids = ["nci","x-ray","xml","samba","jsonlog-16m","osdb","webster"];
    println!("PAIRED estimator, L{lvl}, arm={arm}  (spec vs generic; negative = spec faster)");
    println!("{:<14}{:>10}{:>10}   verdict", "corpus", "mean %", "pos/3");
    let (mut sp, mut gn) = (0,0);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(cap)];
        let mut d = vec![];
        for _ in 0..3 {
            let a1=phase(src,lvl,&arm,false,7); let b1=phase(src,lvl,&arm,true,7);
            let b2=phase(src,lvl,&arm,true,7);  let a2=phase(src,lvl,&arm,false,7);
            d.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
        }
        let mean: f64 = d.iter().sum::<f64>()/d.len() as f64;
        let pos = d.iter().filter(|&&x| x>0.0).count();
        // require |mean| > 1% AND 5/5 or 0/5 agreement to call it
        let v = if mean < -1.0 && pos == 0 { sp+=1; "spec wins" }
                else if mean > 1.0 && pos == 3 { gn+=1; "generic wins" }
                else { "no signal" };
        println!("{id:<14}{mean:>10.2}{pos:>10}   {v}");
    }
    println!("\nspec {sp}   generic {gn}   (of 18)");
    println!("{}", if sp>0 && gn>0 {"SIGN FLIP -> DISPATCH"} else if gn>0 {"generic wins -> turn the specialisation OFF"}
                   else if sp>0 {"spec wins -> CONSTANT ON"} else {"NO SIGNAL -- effect below the noise floor"});
    rusty_zstd::set_bt_spec_arm(true); rusty_zstd::set_dfast_spec_arm(true); rusty_zstd::set_fast_spec_arm(true);
}
