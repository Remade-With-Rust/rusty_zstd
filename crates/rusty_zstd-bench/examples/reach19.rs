//! GATE 4 @ L19 reachability: does find_fast's dispatcher run at all?
fn main() {
    let src = std::fs::read("corpora/data/silesia/xml").unwrap();
    let src = &src[..src.len().min(2*1024*1024)];
    println!("{:<6}{:>16}{:>16}   verdict", "level", "find_fast calls", "find_opt calls");
    for lvl in [1, 3, 19, 22] {
        let _ = rusty_zstd::take_finder_calls();
        let _ = rusty_zstd::compress(src, lvl).unwrap();
        let (f, o) = rusty_zstd::take_finder_calls();
        let v = if f > 0 { "Gate 4 REACHED" } else { "Gate 4 not reached" };
        println!("L{lvl:<5}{f:>16}{o:>16}   {v}");
    }
    println!("\nand with the Gate 4 arm flipped at L19 (default ON, testing OFF):");
    for spec in [true, false] {
        rusty_zstd::set_fast_spec_arm(spec);
        let _ = rusty_zstd::take_finder_calls();
        let z = rusty_zstd::compress(src, 19).unwrap();
        let (f, o) = rusty_zstd::take_finder_calls();
        println!("  spec={spec:<5} find_fast={f} find_opt={o} bytes={}", z.len());
    }
    rusty_zstd::set_fast_spec_arm(true);
}
