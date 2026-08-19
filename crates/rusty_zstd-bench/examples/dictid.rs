fn main() {
    let b = std::fs::read("target/gg.dict").expect("target/gg.dict — run `zstd --train` first");
    let d = rusty_zstd::Dictionary::from_bytes(&b).expect("parse");
    println!("dict {} bytes, id {:#x} ({}), content {} bytes", b.len(), d.id(), d.id(), d.content().len());
    assert!(d.id() != 0, "need a non-zero Dictionary_ID for Gate 3");
}
