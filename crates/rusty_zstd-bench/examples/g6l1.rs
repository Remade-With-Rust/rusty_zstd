//! GATE 6 @ L1 -- keeping the finder's buffers on the frame. Deterministic.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static CALLS: AtomicU64 = AtomicU64::new(0);
static COPIED: AtomicU64 = AtomicU64::new(0);
static LARGE: AtomicU64 = AtomicU64::new(0);
static LGB: AtomicU64 = AtomicU64::new(0);
static ON: AtomicU64 = AtomicU64::new(0);
const BIG: usize = 128 << 10;
struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Relaxed)==1 { CALLS.fetch_add(1,Relaxed);
            if l.size()>=BIG { LARGE.fetch_add(1,Relaxed); LGB.fetch_add(l.size() as u64,Relaxed); } }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self,p:*mut u8,l:Layout){ if ON.load(Relaxed)==1 {CALLS.fetch_add(1,Relaxed);} unsafe{System.dealloc(p,l)} }
    unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{
        if ON.load(Relaxed)==1 { CALLS.fetch_add(1,Relaxed);
            if n>l.size() { COPIED.fetch_add(l.size() as u64,Relaxed); }
            if n>=BIG { LARGE.fetch_add(1,Relaxed); LGB.fetch_add((n-l.size()) as u64,Relaxed); } }
        unsafe{System.realloc(p,l,n)}
    }
}
#[global_allocator] static A: C = C;
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn run(s:&[u8],lvl:i32,on:bool)->(u64,u64,u64,u64,Vec<u8>){
    rusty_zstd::set_finder_scratch_arm(on);
    CALLS.store(0,Relaxed);COPIED.store(0,Relaxed);LARGE.store(0,Relaxed);LGB.store(0,Relaxed);
    ON.store(1,Relaxed);
    let z=rusty_zstd::compress(s,lvl).unwrap();
    ON.store(0,Relaxed);
    (CALLS.load(Relaxed),COPIED.load(Relaxed),LARGE.load(Relaxed),LGB.load(Relaxed),z)
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    let cap:usize=std::env::args().nth(2).and_then(|s|s.parse().ok()).unwrap_or(8<<20);
    println!("GATE 6 @ L{lvl} -- finder buffers kept on the frame ({} MiB board)\n",cap>>20);
    println!("{:<13} | {:>9} {:>9} {:>7} | {:>7} {:>7} | {:>12} {:>12} {:>6}",
        "corpus","off call","on call","call%","off lg","on lg","off lg bytes","on lg bytes","ident");
    println!("{}","-".repeat(104));
    let (mut a0,mut a1,mut g0,mut g1,mut b0,mut b1)=(0u64,0u64,0u64,0u64,0u64,0u64);
    let (mut id_ok,mut n,mut worse)=(0usize,0usize,0usize);
    for id in IDS{
        let Ok(f)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let s=&f[..f.len().min(cap)];
        let _=run(s,lvl,false); let (c0,_p0,l0,y0,z0)=run(s,lvl,false);
        let _=run(s,lvl,true);  let (c1,_p1,l1,y1,z1)=run(s,lvl,true);
        let (c0b,_,_,_,_)=run(s,lvl,false);
        assert_eq!(c0,c0b,"{id}: allocator count NOT deterministic");
        assert!(rusty_zstd::decompress(&z1).unwrap()==s,"{id}: round-trip");
        if z0==z1 {id_ok+=1} else {println!("  {id}: OUTPUT DIFFERS -- DEFECT")}
        if l1>l0 {worse+=1}
        a0+=c0;a1+=c1;g0+=l0;g1+=l1;b0+=y0;b1+=y1;n+=1;
        println!("{:<13} | {:>9} {:>9} {:>6.1}% | {:>7} {:>7} | {:>12} {:>12} {:>6}",
            id,c0,c1,(c1 as f64/c0.max(1) as f64-1.0)*100.0,l0,l1,y0,y1,if z0==z1{"yes"}else{"NO"});
    }
    println!("\n  byte-identical: {id_ok}/{n}   (REQUIRED)");
    println!("  allocator calls  {a0} -> {a1} ({:+.2}%)",(a1 as f64/a0.max(1) as f64-1.0)*100.0);
    println!("  allocations >=128 KiB  {g0} -> {g1} ({:+.2}%), worse on {worse}/{n}",
        (g1 as f64/g0.max(1) as f64-1.0)*100.0);
    println!("  bytes requested >=128 KiB  {b0} -> {b1} ({:+.2}%)",
        (b1 as f64/b0.max(1) as f64-1.0)*100.0);
}
