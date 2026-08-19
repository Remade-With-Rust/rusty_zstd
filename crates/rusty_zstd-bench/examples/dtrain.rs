//! Get a trained dictionary with a NON-ZERO id, cheaply. The first attempt at
//! this ran `train()` with k=0 (a 4-way segment-size sweep) over 3.5 MB of
//! samples and burned 49 minutes without finishing.
use std::time::Instant;
fn main() {
    let x = std::fs::read("corpora/data/silesia/xml").unwrap();
    for &(nsamp, chunk, kb) in &[(64usize, 2048usize, 32usize), (128, 2048, 64), (256, 1024, 64)] {
        let samples: Vec<&[u8]> = x.chunks(chunk).take(nsamp).collect();
        let o = rusty_zstd::TrainOptions { max_dict: kb << 10, dict_id: Some(0x00C0FFEE), ..rusty_zstd::TrainOptions::fastcover() };
        let t = Instant::now();
        match rusty_zstd::train(&samples, o) {
            Ok(b) => {
                let d = rusty_zstd::Dictionary::from_bytes(&b).unwrap();
                println!("n={nsamp} chunk={chunk} max={kb}K -> {} bytes, id {:#x}, {:.2}s", b.len(), d.id(), t.elapsed().as_secs_f64());
                if d.id() != 0 { std::fs::write("target/_g3.dict", &b).unwrap(); println!("  wrote target/_g3.dict"); return; }
            }
            Err(e) => println!("n={nsamp} chunk={chunk} max={kb}K -> ERR {e:?}, {:.2}s", t.elapsed().as_secs_f64()),
        }
    }
}
