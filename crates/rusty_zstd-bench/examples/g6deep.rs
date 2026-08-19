//! GATE 6, one layer down: WHO is doing the reallocating?
//!
//! The payload buffer is now reused, yet L19 still memcpy's 340 MB through
//! `realloc` on a 2 MiB board -- ~19x the input on `x-ray` alone. That is not
//! the payload. This buckets every realloc by the size it is growing TO, so the
//! buffer identifies itself by its growth ladder.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

const NB: usize = 28;
static BUCKET_N: [AtomicU64; NB] = [const { AtomicU64::new(0) }; NB];
static BUCKET_B: [AtomicU64; NB] = [const { AtomicU64::new(0) }; NB];
static ON: AtomicU64 = AtomicU64::new(0);

fn bucket(n: usize) -> usize { ((usize::BITS - n.leading_zeros()) as usize).min(NB - 1) }

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { unsafe { System.alloc(l) } }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if ON.load(Relaxed) == 1 && new > l.size() {
            let b = bucket(new);
            BUCKET_N[b].fetch_add(1, Relaxed);
            BUCKET_B[b].fetch_add(l.size() as u64, Relaxed);
        }
        unsafe { System.realloc(p, l, new) }
    }
}
#[global_allocator]
static A: Counting = Counting;

const IDS: &[&str] = &["x-ray","mozilla","samba","dickens","sao","incomp-32m"];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(2 << 20);
    println!("GATE 6 DEEP @ L{lvl} -- realloc growth ladder ({} MiB board)\n", cap >> 20);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        for b in 0..NB { BUCKET_N[b].store(0, Relaxed); BUCKET_B[b].store(0, Relaxed); }
        ON.store(1, Relaxed);
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        ON.store(0, Relaxed);
        let tot: u64 = (0..NB).map(|b| BUCKET_B[b].load(Relaxed)).sum();
        println!("{id}  total copied {tot} B");
        let mut rows: Vec<(usize,u64,u64)> = (0..NB)
            .map(|b| (b, BUCKET_N[b].load(Relaxed), BUCKET_B[b].load(Relaxed)))
            .filter(|r| r.1 > 0).collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.2));
        for (b, n, by) in rows.iter().take(6) {
            println!("    grow-to ~{:>9} : {:>7} reallocs, {:>12} B copied ({:>5.1}%)",
                fmt(1usize << b.saturating_sub(1)), n, by, *by as f64 / tot.max(1) as f64 * 100.0);
        }
        println!();
    }
}
fn fmt(n: usize) -> String {
    if n >= 1<<20 { format!("{} MiB", n>>20) } else if n >= 1<<10 { format!("{} KiB", n>>10) } else { format!("{n} B") }
}
