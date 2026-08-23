//! DETERMINISTIC census for the inline-execution bricks. No clock.
//!
//! D8a / V1 claimed: "the AVX2 kernel is unreachable from the codec -- the
//! encoder, decoder and streaming API all use `Xxh64::update`, which is
//! scalar." That is a claim about the CALL GRAPH, so it is settled by a COUNT:
//! how many bytes of a real compress+decompress reach the vector kernel.
//!
//! E11 / V2 claimed: `covers` is a redundant O(n) walk over the literals. That
//! is a claim about WORK, so it too is settled by a count: how many literal
//! bytes the deleted walk would have read.
//!
//! Same numbers every run, on any machine, at any load.
const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "versions-16m", "jsonlog-16m",
    "dickens", "mozilla", "samba", "webster", "x-ray", "osdb", "reymont",
];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    rusty_zstd::set_xxh_avx2_arm(true);
    println!("INLINE-EXECUTION CENSUS (L{lvl}) -- counts, not clocks\n");
    println!("{:<14}{:>10}{:>14}{:>14}{:>9}{:>8}{:>14}", "corpus", "MiB", "hybrid B", "scalar B", "hyb%", "calls", "E11 walk B");
    let (mut th, mut ts) = (0u64, 0u64);
    let (mut te11, mut te11c) = (0u64, 0u64);
    let mut te12 = [0u64; 3];
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(32 << 20)];
        let _ = rusty_zstd::xxh_census::take();
        let _ = rusty_zstd::take_e11_walked();
        let _ = rusty_zstd::take_e12_scan();
        let z = rusty_zstd::compress(src, lvl).unwrap();
        let out = rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out, src, "{id} roundtrip");
        let (h, s, c) = rusty_zstd::xxh_census::take();
        let (e11b, e11c) = rusty_zstd::take_e11_walked();
        th += h; ts += s; te11 += e11b; te11c += e11c;
        let e12 = rusty_zstd::take_e12_scan();
        for i in 0..3 { te12[i] += e12[i]; }
        let pct = if h + s == 0 { 0.0 } else { 100.0 * h as f64 / (h + s) as f64 };
        println!("{id:<14}{:>10.1}{h:>14}{s:>14}{pct:>8.1}%{c:>8}{e11b:>14}",
            src.len() as f64 / (1 << 20) as f64);
    }
    let pct = if th + ts == 0 { 0.0 } else { 100.0 * th as f64 / (th + ts) as f64 };
    println!("\nD8a  hybrid {th}  scalar {ts}  ->  {pct:.2}% of checksummed bytes reach the AVX2 kernel");
    println!("E11  the deleted `covers` walk would have read {te11} literal bytes over {te11c} reuse-path blocks");
    println!("E12  limit_nbits: {} calls, {} adjustment steps, {} inner-scan element visits",
        te12[0], te12[2], te12[1]);
    if te12[0] > 0 {
        println!("     = {:.1} steps/call, {:.0} visits/call  (E11 walked {} bytes for comparison)",
            te12[2] as f64 / te12[0] as f64, te12[1] as f64 / te12[0] as f64, te11);
    }
    if te11c > 0 {
        println!("     now {} x 256 = {} histogram reads  ->  {:.0}x less work in that pass",
            te11c, te11c * 256, te11 as f64 / (te11c * 256) as f64);
    }
}
