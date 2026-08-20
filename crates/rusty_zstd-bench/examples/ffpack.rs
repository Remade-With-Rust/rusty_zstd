//! ffanat 5a receipts: the packed Fast tag, decided deterministically.
//!   1. BYTE-IDENTITY across the arm, 18 corpora x L1/L2 (+L3 guard: DFast
//!      must not move). text-32m / versions-16m drive rep_run high enough to
//!      fire the Fast->Lazy switch, exercising the unpack path.
//!   2. ALLOCATOR DELTA: the separate tags array (1 << hash_log bytes) must
//!      disappear from packed frames.
//!   3. FILTER PARITY: probes and hits must be equal across arms -- the tag
//!      compare moved onto the already-loaded line; its decisions must not.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static BYTES: AtomicU64 = AtomicU64::new(0);
static ON: AtomicU64 = AtomicU64::new(0);
struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Relaxed) == 1 { BYTES.fetch_add(l.size() as u64, Relaxed); }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ON.load(Relaxed) == 1 && n > l.size() { BYTES.fetch_add((n - l.size()) as u64, Relaxed); }
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator] static A: C = C;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
#[cfg(feature = "profile")]
fn reads() -> (u64, u64, u64, u64) {
    let (ta, pk) = rusty_zstd::take_tag_reads();
    let (fr, rej) = rusty_zstd::take_tag_rejects();
    (ta, pk, fr, rej)
}
fn run(s: &[u8], lvl: i32, on: bool) -> (Vec<u8>, u64) {
    rusty_zstd::set_fast_pack_arm(on);
    BYTES.store(0, Relaxed); ON.store(1, Relaxed);
    let z = rusty_zstd::compress(s, lvl).unwrap();
    ON.store(0, Relaxed);
    (z, BYTES.load(Relaxed))
}
fn main() {
    // Guard unification (encode.rs dispatch): pack-off now IMPLIES the legacy
    // hash, so a raw pack A/B would compare wide-vs-legacy FINDERS, not the
    // storage transform this board exists to receipt. Pin the wide arm OFF for
    // the whole board; the wide arm has its own byte receipts (ffvers, ffhash).
    rusty_zstd::set_fast_hash_arm(false);
    let (mut same, mut tot) = (0usize, 0usize);
    let (mut boff, mut bon) = (0u64, 0u64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(6 << 20)];
        for lvl in [1, 2, 3] {
            let (a, ba) = run(s, lvl, false);
            let (b, bb) = run(s, lvl, true);
            tot += 1;
            if a == b { same += 1 } else { println!("  MISMATCH {id} L{lvl}: {} vs {}", a.len(), b.len()); }
            assert!(rusty_zstd::decompress(&b).unwrap() == s, "{id} L{lvl} round-trip");
            if lvl <= 2 { boff += ba; bon += bb; }
        }
    }
    println!("BYTE-IDENTICAL across the pack arm: {same}/{tot} cells (L1/L2/L3 x 18)");
    #[cfg(feature = "profile")]
    {
        // executed-path receipt on one heavy corpus at L1
        let f = std::fs::read("corpora/data/silesia/dickens").unwrap();
        let s = &f[..f.len().min(6 << 20)];
        let _ = run(s, 1, false);
        let _ = reads();
        let _ = run(s, 1, false);
        let (ta0, pk0, _f0, r0) = reads();
        let _ = run(s, 1, true);
        let _ = reads();
        let _ = run(s, 1, true);
        let (ta1, pk1, _f1, r1) = reads();
        println!("dickens L1 tag compares: OFF  tag-array {ta0}, packed {pk0}, rejects {r0}");
        println!("dickens L1 tag compares: ON   tag-array {ta1}, packed {pk1}, rejects {r1}");
        println!("  second-cache-line reads removed: {}", ta0 as i64 - ta1 as i64);
    }
    println!("alloc bytes, Fast frames: off {boff} -> on {bon}  ({:+})", bon as i64 - boff as i64);
}
