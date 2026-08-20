//! T1 instrument on the tag-audit routing holes: DFast frames >= 16 MiB (and
//! streaming DFast) ran with NO tag filter because the array fallback was
//! Fast-only. This board prices the fix on FULL-LENGTH corpora, where
//! `enable_packed_tags` refuses the frame and the array route is the only one.
//!
//! A = `set_dfast_tag_arm(false)`  == shipping behavior BEFORE the fix
//! B = `set_dfast_tag_arm(true)`   == the fix (array-form filter)
//!
//! The ledger, per T1's shape:
//!   cost    = one `tags[h]` byte read per nonempty short-slot probe, one tag
//!             write per store, and the `1 << hash_log` allocation
//!   benefit = every rejection avoids a random `src[m]` candidate load, the
//!             gram compare, and a `count_match` that dies below `mls`
//!   bytes   = MUST be identical (the tag derives from the same 4 bytes as
//!             the index; a real match implies an equal tag)
//!
//! Requires `--features profile` for the reject ledger.
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

// The generated >= 16 MiB corpora plus every silesia file that is >= 16 MiB at
// FULL length (mozilla 51M, webster 41M, nci 33M, samba 21M) -- the latter are
// the realistic population the routing hole actually affected.
const IDS: &[&str] = &["versions-16m", "jsonlog-16m", "text-32m", "zeros-32m", "incomp-32m",
    "mozilla", "webster", "nci", "samba"];

fn run(s: &[u8], lvl: i32, arm: bool) -> (Vec<u8>, u64, u64, u64) {
    rusty_zstd::set_dfast_tag_arm(arm);
    #[cfg(feature = "profile")]
    let _ = rusty_zstd::take_tag_rejects();
    BYTES.store(0, Relaxed);
    ON.store(1, Relaxed);
    let z = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
    ON.store(0, Relaxed);
    let ab = BYTES.load(Relaxed);
    #[cfg(feature = "profile")]
    let (rej, nonempty) = rusty_zstd::take_tag_rejects();
    #[cfg(not(feature = "profile"))]
    let (rej, nonempty) = (0u64, 0u64);
    (z, rej, nonempty, ab)
}

fn main() {
    let (mut cells, mut same) = (0usize, 0usize);
    let (mut trej, mut tnon) = (0u64, 0u64);
    println!("corpus         lvl  bytes(A=off)  bytes(B=on)  rejections   nonempty  rej%   alloc delta");
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        assert!(f.len() >= 0x0100_0000, "{id}: board is for frames the pack guard refuses");
        for lvl in [3, 4] {
            let (za, ra, _na, aa) = run(&f, lvl, false);
            let (zb, rb, nb, ab) = run(&f, lvl, true);
            assert!(rusty_zstd::decompress(&zb).unwrap() == f, "{id} L{lvl} round-trip");
            assert_eq!(ra, 0, "{id} L{lvl}: arm OFF must reject nothing");
            cells += 1;
            if za == zb { same += 1; }
            let pct = if nb == 0 { 0.0 } else { 100.0 * rb as f64 / nb as f64 };
            println!(
                "{id:14} L{lvl}  {:12} {:12} {rb:11} {nb:10}  {pct:4.1}%  {:+}",
                za.len(), zb.len(), ab as i64 - aa as i64
            );
            trej += rb;
            tnon += nb;
        }
    }
    println!("BYTE-IDENTICAL across the arm: {same}/{cells} cells");
    println!("TOTAL: {trej} candidate loads avoided of {tnon} nonempty probes ({:.1}%)",
        if tnon == 0 { 0.0 } else { 100.0 * trej as f64 / tnon as f64 });
    assert_eq!(same, cells, "the tag filter must not move bytes");
}
