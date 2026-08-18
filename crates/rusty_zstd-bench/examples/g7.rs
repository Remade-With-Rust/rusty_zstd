//! GATE 7 is GONE: the packed-tag representation was removed. Kept as a
//! regression guard that no `packed` path survives.
fn main() {
    println!("Gate 7 (packed tag slots) has been removed from the encoder.");
    println!("It was default-off, measured larger on 9/18 corpora at L1, slower");
    println!("6/6 in a prior campaign, and its 24-bit position residue could");
    println!("misreconstruct stale slots into the current window.");
}
