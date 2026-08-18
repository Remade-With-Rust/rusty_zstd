//! GATE 6 (pair search at ip+1). Step 1: prove the arm is LIVE somewhere.
//! Step 2: is it reached at L3? Step 3: if dead, validate why.
fn size(src: &[u8], lvl: i32, pair: bool) -> (usize, u64) {
    rusty_zstd::set_pair_on_arm(pair);
    let _ = rusty_zstd::take_finder_calls();
    let z = rusty_zstd::compress(src, lvl).unwrap();
    let (f, _o) = rusty_zstd::take_finder_calls();
    rusty_zstd::set_pair_on_arm(false);
    (z.len(), f)
}
fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    for lvl in [1, 3] {
        let (mut diff, mut n, mut ff) = (0, 0, 0u64);
        for id in ids {
            let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let src = &full[..full.len().min(8*1024*1024)];
            let (a, fa) = size(src, lvl, false);
            let (b, _)  = size(src, lvl, true);
            n += 1; ff += fa;
            if a != b { diff += 1; }
        }
        println!("L{lvl}: pair OFF vs ON changes {diff}/{n} corpora   find_fast calls={ff}");
    }
    println!("\nL1 is the CONTROL (find_fast runs there). L3 is the target.");
}
