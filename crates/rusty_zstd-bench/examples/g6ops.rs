//! GATE 6 deep: three ways to size `find_opt`'s parse-backtrace buffer.
//!   arm 0  reuse only            -- one growth ladder per frame
//!   arm 1  exact  (pre-walk)     -- count the chain, reserve_exact, never grow
//!   arm 2  blanket (n + 1)       -- exact upper bound, no walk, over-reserves
//! Deterministic counters only. The clock's noise floor here is +-24%.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static CALLS: AtomicU64 = AtomicU64::new(0);
static COPIED: AtomicU64 = AtomicU64::new(0);
static LARGE: AtomicU64 = AtomicU64::new(0);
static RESV: AtomicU64 = AtomicU64::new(0);
static ON: AtomicU64 = AtomicU64::new(0);
const LARGE_MIN: usize = 128 << 10;

struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Relaxed) == 1 {
            CALLS.fetch_add(1, Relaxed);
            RESV.fetch_add(l.size() as u64, Relaxed);
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
            if new > l.size() { COPIED.fetch_add(l.size() as u64, Relaxed); }
            RESV.fetch_add(new.saturating_sub(l.size()) as u64, Relaxed);
            if new >= LARGE_MIN { LARGE.fetch_add(1, Relaxed); }
        }
        unsafe { System.realloc(p, l, new) }
    }
}
#[global_allocator]
static A: C = C;

const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];

fn run(s: &[u8], lvl: i32, arm: u8) -> (u64, u64, u64, u64, Vec<u8>) {
    rusty_zstd::set_opt_ops_arm(arm);
    CALLS.store(0,Relaxed); COPIED.store(0,Relaxed); LARGE.store(0,Relaxed); RESV.store(0,Relaxed);
    ON.store(1, Relaxed);
    let z = rusty_zstd::compress(s, lvl).unwrap();
    ON.store(0, Relaxed);
    (CALLS.load(Relaxed), COPIED.load(Relaxed), LARGE.load(Relaxed), RESV.load(Relaxed), z)
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 6 DEEP @ L{lvl} -- sizing find_opt's backtrace buffer ({} MiB board)\n", cap>>20);
    println!("{:<13} | {:>12} {:>12} {:>12} | {:>7} {:>7} {:>7} | {:>12} {:>12}",
        "corpus","copy reuse","copy exact","copy blank","lg reu","lg exa","lg bla","bytes exact","bytes blank");
    println!("{}", "-".repeat(120));
    let (mut c0,mut c1,mut c2)=(0u64,0u64,0u64);
    let (mut l0,mut l1,mut l2)=(0u64,0u64,0u64);
    let (mut r1,mut r2)=(0u64,0u64);
    let (mut ident,mut n)=(0usize,0usize);
    let (mut exact_wins, mut exact_loses) = (0usize, 0usize);
    for id in IDS {
        let Ok(f)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s=&f[..f.len().min(cap)];
        let _=run(s,lvl,0);
        let (a0,p0,g0,_,z0)=run(s,lvl,0);
        let _=run(s,lvl,1);
        let (a1,p1,g1,v1,z1)=run(s,lvl,1);
        let _=run(s,lvl,2);
        let (_a2,p2,g2,v2,z2)=run(s,lvl,2);
        let (a0b,_,_,_,_)=run(s,lvl,0);
        assert_eq!(a0,a0b,"{id}: allocator count NOT deterministic");
        assert_eq!(a1>0,true);
        if z0==z1 && z1==z2 { ident+=1 } else { println!("  {id}: OUTPUT DIFFERS -- DEFECT"); }
        if p1 < p0 { exact_wins+=1 } else if p1 > p0 { exact_loses+=1 }
        c0+=p0; c1+=p1; c2+=p2; l0+=g0; l1+=g1; l2+=g2; r1+=v1; r2+=v2; n+=1;
        println!("{:<13} | {:>12} {:>12} {:>12} | {:>7} {:>7} {:>7} | {:>12} {:>12}",
            id,p0,p1,p2,g0,g1,g2,v1,v2);
    }
    println!("\n  byte-identical across all three arms: {ident}/{n}");
    println!("  bytes copied   reuse {c0}  ->  exact {c1} ({:+.2}%)  |  blanket {c2} ({:+.2}%)",
        (c1 as f64/c0.max(1) as f64-1.0)*100.0, (c2 as f64/c0.max(1) as f64-1.0)*100.0);
    println!("  large allocs   reuse {l0}  ->  exact {l1} ({:+})  |  blanket {l2} ({:+})",
        l1 as i64-l0 as i64, l2 as i64-l0 as i64);
    println!("  TOTAL BYTES REQUESTED FROM THE ALLOCATOR: exact {r1}  vs  blanket {r2} ({:+.2}%)",
        (r2 as f64/r1.max(1) as f64-1.0)*100.0);
    println!("  exact beats reuse on copies: {exact_wins}/{n}, loses on {exact_loses}/{n}");
}
