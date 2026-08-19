//! Name the remaining large reallocator. Captures a backtrace for every realloc
//! growing past 1 MiB, behind a reentrancy guard (capturing a backtrace itself
//! allocates, so without the guard the allocator recurses into itself).
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;

static ON: AtomicU64 = AtomicU64::new(0);
static HITS: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }
const BIG: usize = 1 << 20;

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { unsafe { System.alloc(l) } }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if ON.load(Relaxed) == 1 && new >= BIG && new > l.size() {
            let go = BUSY.with(|b| if b.get() { false } else { b.set(true); true });
            if go {
                let bt = std::backtrace::Backtrace::force_capture().to_string();
                let frame = bt.lines()
                    .filter(|s| s.contains("rusty_zstd") && !s.contains("g6who") && !s.contains("Counting"))
                    .nth(1).unwrap_or("?").trim().to_string();
                if let Ok(mut h) = HITS.lock() { h.push((new, frame)); }
                BUSY.with(|b| b.set(false));
            }
        }
        unsafe { System.realloc(p, l, new) }
    }
}
#[global_allocator]
static A: Counting = Counting;

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = 2 << 20;
    for id in ["x-ray", "mozilla", "dickens"] {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        HITS.lock().unwrap().clear();
        ON.store(1, Relaxed);
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        ON.store(0, Relaxed);
        let h = HITS.lock().unwrap();
        println!("{id} @ L{lvl}: {} reallocs past {} MiB", h.len(), BIG >> 20);
        for (n, fr) in h.iter().take(8) {
            println!("   -> {:>9} B   {}", n, fr);
        }
        println!();
    }
}
