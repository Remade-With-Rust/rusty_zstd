//! Block-type census of a .zst file: `cargo run --example census -- f.zst`
fn main() {
    for p in std::env::args().skip(1) {
        let src = std::fs::read(&p).expect("read");
        match rusty_zstd::frame_block_census(&src) {
            Ok(c) => println!(
                "{p}\n  raw={} ({} B)  rle={} (regen {} B)  compressed={} (payload {} B)  file={} B",
                c.raw, c.raw_bytes, c.rle, c.rle_regen, c.compressed, c.compressed_payload, src.len()
            ),
            Err(e) => println!("{p}: census error {e:?}"),
        }
    }
}
