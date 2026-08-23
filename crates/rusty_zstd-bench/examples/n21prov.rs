//! N21 provenance check. codec-measurement §9: "bitstream PROVENANCE is content."
//! Our own encoder rarely selects Predefined mode; a foreign encoder may not.
//! Decode C-zstd-produced frames and count the RFC-constant table rebuilds.
fn main(){
    let dir = std::env::args().nth(1).expect("dir");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e=="zst").unwrap_or(false)).collect();
    files.sort();
    let _ = rusty_zstd::take_n21_predef();
    let (mut frames, mut bytes) = (0usize, 0usize);
    for f in &files {
        let z = std::fs::read(f).unwrap();
        let out = rusty_zstd::decompress(&z).expect("decode C frame");
        bytes += out.len(); frames += 1;
    }
    let n = rusty_zstd::take_n21_predef();
    println!("C-zstd frames decoded: {frames}, {} MiB", bytes>>20);
    println!("N21 predefined rebuilds: {n}  ({:.2} per MiB)", n as f64/((bytes>>20) as f64));
}
