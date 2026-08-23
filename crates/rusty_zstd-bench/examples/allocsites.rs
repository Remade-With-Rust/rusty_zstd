//! allocation-census step 1b: ATTRIBUTION. Which sites make up the ~78
//! allocations per block?
//!
//! Backtrace-sampled rather than tag-instrumented, so it FINDS sites instead of
//! confirming the ones already suspected. Build with -Cdebuginfo=1 or the
//! frames will not resolve.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static ON: AtomicUsize = AtomicUsize::new(0);
static N: AtomicUsize = AtomicUsize::new(0);
static MIN: AtomicUsize = AtomicUsize::new(usize::MAX);
static SITES: Mutex<Option<HashMap<String, (u64, u64)>>> = Mutex::new(None);
thread_local! { static REENTRY: Cell<bool> = const { Cell::new(false) }; }

struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 {
            let n = N.fetch_add(1, Ordering::Relaxed);
            // sample: backtrace capture allocates, so guard against reentry
            let big = l.size() >= MIN.load(Ordering::Relaxed);
            if big || n % 37 == 0 {
                REENTRY.with(|r| {
                    if !r.get() {
                        r.set(true);
                        let bt = std::backtrace::Backtrace::force_capture().to_string();
                        let mut key = String::from("?");
                        for line in bt.lines() {
                            let t = line.trim();
                            if let Some(p) = t.find("rusty_zstd::") {
                                let s = &t[p..];
                                let end = s.find(' ').unwrap_or(s.len());
                                let cand = &s[..end];
                                if !cand.contains("allocsites") {
                                    key = cand.trim_end_matches("::{{closure}}").to_string();
                                    break;
                                }
                            }
                        }
                        let mut g = SITES.lock().unwrap();
                        let m = g.get_or_insert_with(HashMap::new);
                        let e = m.entry(key).or_insert((0, 0));
                        e.0 += 1;
                        e.1 += l.size() as u64;
                        r.set(false);
                    }
                });
            }
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, ns: usize) -> *mut u8 {
        unsafe { System.realloc(p, l, ns) }
    }
}
#[global_allocator]
static A: C = C;

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    if let Ok(m) = std::env::var("ALLOC_MIN") { if let Ok(v) = m.parse() { MIN.store(v, Ordering::Relaxed); } }
    let full = std::fs::read("corpora/data/silesia/dickens").expect("corpus");
    let src = &full[..full.len().min(8 << 20)];
    ON.store(1, Ordering::Relaxed);
    let _ = rusty_zstd::compress(src, lvl).unwrap();
    ON.store(0, Ordering::Relaxed);
    let total = N.load(Ordering::Relaxed);
    let g = SITES.lock().unwrap();
    let m = g.as_ref().unwrap();
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by_key(|(_, (c, _))| std::cmp::Reverse(*c));
    let sampled: u64 = m.values().map(|(c, _)| c).sum();
    println!("L{lvl}: {total} allocations, {sampled} sampled (1 in 37)\n");
    println!("{:>9}{:>10}{:>14}  {}", "hits", "hits", "bytes", "site");
    for (k, (c, b)) in v.iter().take(22) {
        println!("{:>9}{:>10}{:>14}  {}", c, c, b, k);
    }
}
