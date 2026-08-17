//! Which BODY runs? Probe counts and bytes are identical between the two
//! find_dfast implementations by construction, so only a call counter can show
//! that the arm actually switches functions.
fn main() {
    let src = std::fs::read("corpora/data/silesia/xml").unwrap();
    for spec in [false, true] {
        rusty_zstd::set_dfast_spec_arm(spec);
        let _ = rusty_zstd::take_dfast_calls();          // clear
        let z = rusty_zstd::compress(&src, 3).unwrap();
        let (sp, rt) = rusty_zstd::take_dfast_calls();
        println!("arm spec={spec:<5}  specialised body called {sp:>5}x   runtime body called {rt:>5}x   bytes={}", z.len());
    }
    rusty_zstd::set_dfast_spec_arm(true);
}
