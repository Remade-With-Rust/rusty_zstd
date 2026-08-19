//! GATE 3 safety: what happens when a dictionary-dependent frame is decoded
//! WITHOUT the dictionary? A clean error is required; silent garbage is not.
use rusty_zstd::{CompressOptions, Dictionary};
fn main() {
    let raw = std::fs::read("target/_g3.dict").unwrap();
    let d = Dictionary::from_bytes(&raw).unwrap();
    let src = std::fs::read("corpora/data/silesia/xml").unwrap();
    let src = &src[..1 << 20];
    for (label, w) in [("write-id", true), ("no-id", false)] {
        let z = rusty_zstd::compress_using_dict_with(src, &d, CompressOptions { level: 3, checksum: true }, w).unwrap();
        let bare = rusty_zstd::decompress(&z);
        let wrong = {
            let mut v = raw.clone();
            // same id, DIFFERENT content -> must not decode to the original
            let n = v.len();
            for b in v[n/2..].iter_mut() { *b ^= 0xFF; }
            Dictionary::from_bytes(&v).ok().and_then(|w| rusty_zstd::decompress_using_dict(&z, &w).ok())
        };
        println!("{label:<9} no-dict decode: {:<28} wrong-dict decode: {}",
            match &bare { Ok(v) if v == src => "WRONG: decoded correctly?!".into(),
                          Ok(v) => format!("produced {} bytes (no error)", v.len()),
                          Err(e) => format!("clean error {e:?}") },
            match &wrong { Some(v) if v == src => "matched (content unused?)".to_string(),
                           Some(v) => format!("produced {} bytes", v.len()),
                           None => "clean error".to_string() });
    }
}
