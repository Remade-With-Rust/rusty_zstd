//! allocation-census step 1a: allocations per BLOCK, by differencing.
//!
//! The census gives allocations per MiB. If the count scales linearly with
//! BLOCK COUNT, the COUNT problem is per-block plumbing and N11/N16/N17/N18 are
//! probably one scratch struct rather than four bricks. Differencing two input
//! sizes cancels the fixed per-frame cost, so this needs no attribution.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
static ON: AtomicUsize = AtomicUsize::new(0);
static N: AtomicU64 = AtomicU64::new(0);
struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 { N.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, ns: usize) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 { N.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.realloc(p, l, ns) }
    }
}
#[global_allocator]
static A: C = C;
fn main() {
    let full = std::fs::read("corpora/data/silesia/dickens")
        .or_else(|_| std::fs::read("corpora/data/silesia/webster")).expect("corpus");
    println!("{:<10}{:>10}{:>14}{:>16}{:>16}", "level", "MiB", "allocations", "per 128K block", "per MiB");
    for lvl in [1i32, 3, 9, 19] {
        let mut pts = vec![];
        for mib in [1usize, 2, 4, 8] {
            let src = &full[..full.len().min(mib << 20)];
            if src.len() < (mib << 20) { continue }
            N.store(0, Ordering::Relaxed);
            ON.store(1, Ordering::Relaxed);
            let _ = rusty_zstd::compress(src, lvl).unwrap();
            ON.store(0, Ordering::Relaxed);
            pts.push((mib, N.load(Ordering::Relaxed)));
        }
        // difference the largest pair: cancels the fixed per-frame allocations
        if pts.len() >= 2 {
            let (m0, n0) = pts[0];
            let (m1, n1) = pts[pts.len() - 1];
            let blocks = ((m1 - m0) << 20) as f64 / (128.0 * 1024.0);
            let per_block = (n1 - n0) as f64 / blocks;
            for (m, n) in &pts {
                println!("{:<10}{:>10}{:>14}{:>16}{:>16.0}", format!("L{lvl}"), m, n, "", *n as f64 / *m as f64);
            }
            println!("{:<10}{:>10}{:>14}{:>16.1}{:>16}", "", "marginal", n1 - n0, per_block, "");
        }
    }
}
