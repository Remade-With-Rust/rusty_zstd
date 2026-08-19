//! GATE 6 @ L3 -- payload reserve, measured DETERMINISTICALLY.
//!
//! The clock cannot decide this cell. A null arm (`g6null`, payload_arm(true)
//! against payload_arm(true)) reads up to +-24.15% on identical code, and every
//! candidate signal -- including `incomp-32m`'s apparently-stable +9..13% --
//! lies inside that band.
//!
//! So measure the thing the gate actually controls: allocator traffic. A
//! counting `GlobalAlloc` is exactly reproducible run to run, so the numbers
//! below are facts about the program, not about the machine's mood.
//!
//! Three counters, because they price different CPU work:
//!   * calls      -- every alloc/realloc/dealloc is a trip through the allocator
//!   * copied     -- bytes memcpy'd by `realloc` when a Vec doubles. Pure waste.
//!   * large      -- allocations >= 128 KiB. On Windows these leave the low
//!                   fragmentation heap and become VirtualAlloc, i.e. a syscall
//!                   and a page-table edit. This is the reserve's exposure: it
//!                   asks for `block.len()` = BLOCKSIZE_MAX = 128 KiB exactly.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static CALLS: AtomicU64 = AtomicU64::new(0);
static COPIED: AtomicU64 = AtomicU64::new(0);
static LARGE: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static ON: AtomicU64 = AtomicU64::new(0);
const LARGE_MIN: usize = 128 << 10;

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Relaxed) == 1 {
            CALLS.fetch_add(1, Relaxed);
            BYTES.fetch_add(l.size() as u64, Relaxed);
            if l.size() >= LARGE_MIN { LARGE.fetch_add(1, Relaxed); }
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if ON.load(Relaxed) == 1 { CALLS.fetch_add(1, Relaxed); }
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if ON.load(Relaxed) == 1 {
            CALLS.fetch_add(1, Relaxed);
            // realloc that grows must copy the live bytes if it cannot extend
            COPIED.fetch_add(l.size() as u64, Relaxed);
            BYTES.fetch_add(new.saturating_sub(l.size()) as u64, Relaxed);
            if new >= LARGE_MIN { LARGE.fetch_add(1, Relaxed); }
        }
        unsafe { System.realloc(p, l, new) }
    }
}
#[global_allocator]
static A: Counting = Counting;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];

fn measure(src: &[u8], lvl: i32, arm: bool) -> (u64, u64, u64, u64, usize) {
    rusty_zstd::set_payload_arm(arm);
    CALLS.store(0, Relaxed); COPIED.store(0, Relaxed);
    LARGE.store(0, Relaxed); BYTES.store(0, Relaxed);
    ON.store(1, Relaxed);
    let z = rusty_zstd::compress(src, lvl).unwrap();
    ON.store(0, Relaxed);
    (CALLS.load(Relaxed), COPIED.load(Relaxed), LARGE.load(Relaxed), BYTES.load(Relaxed), z.len())
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 6 @ L{lvl} -- payload reserve, DETERMINISTIC allocator traffic ({} MiB cap)", cap >> 20);
    println!("  large = allocations >= {} KiB (VirtualAlloc territory on Windows)\n", LARGE_MIN >> 10);
    println!("{:<13} | {:>9} {:>9} {:>7} | {:>12} {:>12} {:>8} | {:>7} {:>7} {:>7}",
        "corpus", "off call", "on call", "call%", "off copied", "on copied", "copy%", "off lg", "on lg", "ident");
    println!("{}", "-".repeat(112));

    let (mut tc_off, mut tc_on, mut cp_off, mut cp_on, mut lg_off, mut lg_on) = (0u64,0u64,0u64,0u64,0u64,0u64);
    let (mut ident, mut n) = (0usize, 0usize);
    let (mut call_win, mut call_loss, mut lg_worse) = (0usize, 0usize, 0usize);

    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        // warm: first call allocates one-off scratch that would skew the count
        let _ = measure(s, lvl, false);
        let (c0, p0, l0, _b0, z0) = measure(s, lvl, false);
        let _ = measure(s, lvl, true);
        let (c1, p1, l1, _b1, z1) = measure(s, lvl, true);
        // determinism self-check: the same arm twice must give the same count
        let (c0b, _, _, _, _) = measure(s, lvl, false);
        assert_eq!(c0, c0b, "{id}: allocator count NOT deterministic");
        let same = z0 == z1;
        if same { ident += 1 }
        if c1 < c0 { call_win += 1 } else if c1 > c0 { call_loss += 1 }
        if l1 > l0 { lg_worse += 1 }
        tc_off += c0; tc_on += c1; cp_off += p0; cp_on += p1; lg_off += l0; lg_on += l1;
        n += 1;
        println!("{:<13} | {:>9} {:>9} {:>6.1}% | {:>12} {:>12} {:>7.1}% | {:>7} {:>7} {:>7}",
            id, c0, c1, (c1 as f64/c0.max(1) as f64 - 1.0)*100.0,
            p0, p1, (p1 as f64/p0.max(1) as f64 - 1.0)*100.0, l0, l1,
            if same { "yes" } else { "NO" });
    }
    println!("\n  byte-identical output: {ident}/{n}  (REQUIRED -- this is an allocation arm)");
    println!("  allocator calls  {tc_off} -> {tc_on}  ({:+.2}%)   {call_win} corpora fewer / {call_loss} more",
        (tc_on as f64/tc_off.max(1) as f64 - 1.0)*100.0);
    println!("  bytes memcpy'd by realloc  {cp_off} -> {cp_on}  ({:+.2}%)",
        (cp_on as f64/cp_off.max(1) as f64 - 1.0)*100.0);
    println!("  large (>={} KiB) allocations  {lg_off} -> {lg_on}  ({:+}), worse on {lg_worse}/{n}",
        LARGE_MIN >> 10, lg_on as i64 - lg_off as i64);
}
