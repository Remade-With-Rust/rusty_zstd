//! Find the cheapest `train()` option set that yields a parseable dict with id != 0.
use std::time::Instant;
fn main() {
    let seed: Vec<Vec<u8>> = ["samba", "xml", "webster"]
        .iter()
        .filter_map(|id| std::fs::read(format!("corpora/data/silesia/{id}")).ok())
        .map(|f| f[..f.len().min(1 << 20)].to_vec())
        .collect();
    for &(nsamp, chunk) in &[(24usize, 4096usize), (64, 4096), (128, 8192)] {
        let samples: Vec<&[u8]> = seed.iter().flat_map(|s| s.chunks(chunk).take(nsamp)).collect();
        for &(k, accel, maxd) in &[(200u32, 1u32, 16usize << 10), (200, 1, 64 << 10), (0, 1, 64 << 10), (500, 1, 64 << 10)] {
            let o = rusty_zstd::TrainOptions { max_dict: maxd, dict_id: Some(0x00C0FFEE), k, accel, ..rusty_zstd::TrainOptions::fastcover() };
            let t = Instant::now();
            let r = rusty_zstd::train(&samples, o);
            let el = t.elapsed().as_secs_f64();
            match r {
                Ok(b) => {
                    let d = rusty_zstd::Dictionary::from_bytes(&b);
                    println!("n={} chunk={} k={} accel={} maxd={}K -> {} bytes, id {:?}, {:.2}s",
                        samples.len(), chunk, k, accel, maxd >> 10, b.len(),
                        d.as_ref().map(|d| d.id()), el);
                }
                Err(e) => println!("n={} chunk={} k={} accel={} maxd={}K -> ERR {:?}, {:.2}s", samples.len(), chunk, k, accel, maxd >> 10, e, el),
            }
        }
    }
}
