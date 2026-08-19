//! GATE 6 @ L1: name every allocator call >= 128 KiB by call site.
//!
//! `payload` was doing one VirtualAlloc-class request per block until Gate 6
//! kept the buffer. This asks whether anything ELSE on the L1 path still is.
//! Counts ALLOC (not just realloc), since a fresh per-block `Vec::with_capacity`
//! never reallocs -- it allocates and frees, which is the cost that hides from
//! a realloc-only counter.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::collections::BTreeMap;

static ON: AtomicU64 = AtomicU64::new(0);
static HITS: Mutex<BTreeMap<String, (u64, u64)>> = Mutex::new(BTreeMap::new());
thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }
const BIG: usize = 128 << 10;

fn note(sz: usize) {
    let go = BUSY.with(|b| if b.get() { false } else { b.set(true); true });
    if !go { return }
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    let frame = bt.lines()
        .filter(|s| s.contains("rusty_zstd::") && !s.contains("g6l1who"))
        .map(|s| s.trim().to_string())
        .find(|s| !s.contains("note") && !s.contains("Counting"))
        .unwrap_or_else(|| "?".into());
    if let Ok(mut h) = HITS.lock() {
        // key by call site AND rounded size -- two buffers in one function are
        // two different findings, and the size is what names them.
        let e = h.entry(format!("{frame}  [{sz} B]")).or_insert((0, 0));
        e.0 += 1; e.1 += sz as u64;
    }
    BUSY.with(|b| b.set(false));
}

struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Relaxed) == 1 && l.size() >= BIG { note(l.size()); }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ON.load(Relaxed) == 1 && n >= BIG && n > l.size() { note(n); }
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: C = C;

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    for id in ["mr", "samba", "dickens", "smallmsg-8m"] {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        HITS.lock().unwrap().clear();
        ON.store(1, Relaxed);
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        ON.store(0, Relaxed);
        let h = HITS.lock().unwrap();
        let blocks = s.len().div_ceil(128 << 10);
        println!("{id} @ L{lvl}  ({} KiB in, ~{blocks} blocks of 128 KiB)", s.len() >> 10);
        let mut v: Vec<_> = h.iter().collect();
        v.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
        for (fr, (n, by)) in v.iter().take(8) {
            println!("   {:>5} allocs >=128 KiB, {:>11} B   {}", n, by, fr);
        }
        println!();
    }
}
