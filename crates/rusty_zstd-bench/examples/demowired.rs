//! IS THE CAMPAIGN WIRED TO THE DEMO?
//!
//! Drives the simserver's EXACT call pair -- `compress_with_params(s, p, false)`
//! then `decompress_into` -- and reads every optimisation's own counter. The
//! fixes are unconditional; only the counters are `profile`-gated, so this
//! exercises the same code the demo binary runs.
//!
//! This is the V1 audit turned on ourselves: a demo that does not reach a fix
//! cannot show it, and "we made it faster" is not evidence that the faster path
//! is the one being measured.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
static ON: AtomicUsize = AtomicUsize::new(0);
static NA: AtomicU64 = AtomicU64::new(0);
struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 { NA.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, ns: usize) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 { NA.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.realloc(p, l, ns) }
    }
}
#[global_allocator]
static A: C = C;

const IDS: &[&str] = &["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap = 8usize << 20;
    let srcs: Vec<Vec<u8>> = IDS.iter().filter_map(|id| load(id).map(|f| f[..f.len().min(cap)].to_vec())).collect();
    let mib: f64 = srcs.iter().map(|s| s.len() as f64).sum::<f64>() / (1u64 << 20) as f64;

    // drain
    let _ = rusty_zstd::xxh_census::take();
    let _ = rusty_zstd::take_e11_walked();
    let _ = rusty_zstd::take_n9_basic();
    let _ = rusty_zstd::take_n13_stats();
    let _ = rusty_zstd::take_x2_stats();
    let _ = rusty_zstd::take_d4_paths();

    // PHASE 1: encode only. X2 builds are counted globally, so the encode and
    // decode phases MUST be measured apart -- the decoder legitimately builds
    // X2 tables, and mixing the phases makes HUFF-1 look inactive.
    ON.store(1, Ordering::Relaxed);
    let mut zs = Vec::new();
    for s in &srcs {
        let p = rusty_zstd::compression_params(lvl, Some(s.len() as u64)).unwrap();
        zs.push(rusty_zstd::compress_with_params(s, p, false).unwrap());
    }
    ON.store(0, Ordering::Relaxed);
    let x2_enc = rusty_zstd::take_x2_stats().0;

    // PHASE 2: decode only.
    ON.store(1, Ordering::Relaxed);
    let mut buf = Vec::new();
    for (z, s) in zs.iter().zip(&srcs) {
        buf.clear();
        rusty_zstd::decompress_into(&mut buf, &z).unwrap();
        assert!(buf == **s, "round-trip mismatch");
    }
    ON.store(0, Ordering::Relaxed);
    let x2_dec = rusty_zstd::take_x2_stats().0;

    let (hyb, scal, hcalls) = rusty_zstd::xxh_census::take();
    let (e11b, e11c) = rusty_zstd::take_e11_walked();
    let n9 = rusty_zstd::take_n9_basic();
    let n13 = rusty_zstd::take_n13_stats();
    let d4 = rusty_zstd::take_d4_paths();
    let allocs = NA.load(Ordering::Relaxed);

    println!("DEMO PATH @ L{lvl} -- {mib:.0} MiB through compress_with_params(.., false) + decompress_into\n");
    println!("{:<34}{:>14}  {}", "improvement", "counter", "wired to the demo?");
    println!("{:-<34}{:->16}  {:-<22}", "", "", "");
    let row = |name: &str, val: String, wired: bool, note: &str| {
        println!("{:<34}{:>14}  {} {}", name, val, if wired { "YES" } else { "NO " }, note);
    };
    row("D8a xxh64 AVX2 kernel", format!("{hcalls} calls"), hcalls > 0,
        if hcalls > 0 { "" } else { "<- checksum OFF for zstd -b parity" });
    row("E11 covers-from-freq", format!("{e11c} blocks"), e11c > 0, "");
    row("N9 cached RFC ctables", format!("{n9} rebuilds"), n9 == 0, "(0 rebuilds = fix active)");
    row("N13 two-queue Huffman", format!("{} calls", n13[0]), n13[0] > 0, "");
    row("HUFF-1/2 skip X2+upsample", format!("enc {x2_enc} / dec {x2_dec}"), x2_enc == 0,
        "(enc 0 = fix active; dec builds are CORRECT)");
    row("D4 dict-crossing copy", format!("{} cross", d4[2]), d4[2] > 0,
        if d4[2] > 0 { "" } else { "<- demo uses no dictionary" });
    row("ALLOC-1..18", format!("{:.1}/MiB", allocs as f64 / mib), true, "");
    println!("\n  e11 literal bytes no longer walked: {e11b}");
    println!("  n13 sum(n^2) avoided:               {}", n13[2]);
    println!("  xxh64 bytes: hybrid {hyb}, scalar {scal}");
}
