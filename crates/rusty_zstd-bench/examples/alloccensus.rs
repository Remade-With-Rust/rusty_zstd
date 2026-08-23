//! ALLOCATION CENSUS — the opening instrument for the allocation campaign.
//!
//! `inline-execution.md` opened with a ymm/xmm census and that one number
//! ("491 ymm against 27,862 xmm") set the whole plan's direction. This is the
//! equivalent for §12.2: count every allocation on the ENCODE and DECODE paths
//! separately, bucketed by size, per level. Deterministic — a count, not a clock.
//!
//! Encode and decode must be counted separately: they have different owners,
//! different recycling machinery (W25/W26) and different fixes.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static ON: AtomicUsize = AtomicUsize::new(0);
static N: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
const NB: usize = 9;
static BUCK: [AtomicU64; NB] = [
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0),
];
static BBYTES: [AtomicU64; NB] = [
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0),
];
const LAB: [&str; NB] = [
    "<64", "64..255", "256..1K", "1K..4K", "4K..16K", "16K..64K",
    "64K..256K", "256K..1M", ">=1M",
];
fn bucket(s: usize) -> usize {
    match s {
        0..=63 => 0, 64..=255 => 1, 256..=1023 => 2, 1024..=4095 => 3,
        4096..=16383 => 4, 16384..=65535 => 5, 65536..=262143 => 6,
        262144..=1048575 => 7, _ => 8,
    }
}
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 {
            N.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
            let b = bucket(l.size());
            BUCK[b].fetch_add(1, Ordering::Relaxed);
            BBYTES[b].fetch_add(l.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, ns: usize) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 {
            N.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(ns as u64, Ordering::Relaxed);
            let b = bucket(ns);
            BUCK[b].fetch_add(1, Ordering::Relaxed);
            BBYTES[b].fetch_add(ns as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(p, l, ns) }
    }
}
#[global_allocator]
static A: Counting = Counting;

const IDS: &[&str] = &["reymont","dickens","webster","mr","nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn reset() {
    N.store(0, Ordering::Relaxed); BYTES.store(0, Ordering::Relaxed);
    for b in 0..NB { BUCK[b].store(0, Ordering::Relaxed); BBYTES[b].store(0, Ordering::Relaxed); }
}
fn report(phase: &str, lvl: i32, mib: f64) {
    let n = N.load(Ordering::Relaxed);
    let by = BYTES.load(Ordering::Relaxed);
    println!("\n### {phase} @ L{lvl} — {n} allocations, {:.1} MiB requested, over {mib:.0} MiB of data",
        by as f64 / (1u64 << 20) as f64);
    println!("    {:.1} allocations per MiB", n as f64 / mib);
    println!("    {:<12}{:>12}{:>10}{:>14}{:>9}", "size class", "count", "count%", "bytes", "bytes%");
    for b in 0..NB {
        let c = BUCK[b].load(Ordering::Relaxed);
        if c == 0 { continue }
        let bb = BBYTES[b].load(Ordering::Relaxed);
        println!("    {:<12}{:>12}{:>9.1}%{:>14}{:>8.1}%", LAB[b], c,
            100.0 * c as f64 / n as f64, bb, 100.0 * bb as f64 / by as f64);
    }
}
fn main() {
    let cap = 8usize << 20;
    let srcs: Vec<Vec<u8>> = IDS.iter().filter_map(|id| load(id).map(|f| f[..f.len().min(cap)].to_vec())).collect();
    let mib: f64 = srcs.iter().map(|s| s.len() as f64).sum::<f64>() / (1u64<<20) as f64;
    for lvl in [1i32, 3, 9, 19] {
        // ---- ENCODE ----
        reset(); ON.store(1, Ordering::Relaxed);
        let zs: Vec<Vec<u8>> = srcs.iter().map(|s| rusty_zstd::compress(s, lvl).unwrap()).collect();
        ON.store(0, Ordering::Relaxed);
        report("ENCODE", lvl, mib);
        // ---- DECODE ----
        reset(); ON.store(1, Ordering::Relaxed);
        for (z, s) in zs.iter().zip(&srcs) { assert_eq!(rusty_zstd::decompress(z).unwrap().len(), s.len()); }
        ON.store(0, Ordering::Relaxed);
        report("DECODE", lvl, mib);
    }
}
