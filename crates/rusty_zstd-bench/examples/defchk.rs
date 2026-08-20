//! Is the finder-scratch win actually ON in a build nobody configures?
//! Measures realloc traffic with the arm UNTOUCHED (shipping default) against
//! the arm explicitly disabled.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static COPIED: AtomicU64 = AtomicU64::new(0);
static ON: AtomicU64 = AtomicU64::new(0);
struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { unsafe { System.alloc(l) } }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ON.load(Relaxed) == 1 && n > l.size() { COPIED.fetch_add(l.size() as u64, Relaxed); }
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator] static A: C = C;
const IDS: &[&str] = &["dickens","samba","webster","mozilla","x-ray","sao","mr","osdb"];
fn run(lvl: i32) -> u64 {
    let mut t = 0u64;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(8 << 20)];
        COPIED.store(0, Relaxed); ON.store(1, Relaxed);
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        ON.store(0, Relaxed);
        t += COPIED.load(Relaxed);
    }
    t
}
fn main() {
    for lvl in [5, 9, 13] {
        let dflt = run(lvl);                              // arm never touched
        rusty_zstd::set_finder_scratch_arm(false);
        let off = run(lvl);
        rusty_zstd::set_finder_scratch_arm(true);
        let on = run(lvl);
        println!("L{lvl:<3} default {dflt:>12}   explicit-on {on:>12}   explicit-off {off:>12}   default==on: {}",
            dflt == on);
    }
}
