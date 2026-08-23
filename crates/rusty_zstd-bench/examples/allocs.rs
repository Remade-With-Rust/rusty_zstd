//! Does W26 actually recycle? Count FSE decode-table ALLOCATIONS during decode
//! by wrapping the global allocator and filtering to table-sized requests.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
static N: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static BUCK: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
static ON: AtomicU64 = AtomicU64::new(0);
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // FseEntry is 4 bytes; tables are 2^5..2^9 entries = 128..2048 bytes.
        if ON.load(Ordering::Relaxed) == 1 {
            N.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
            let b = match l.size() {
                128..=199 => 0, 200..=299 => 1, 300..=511 => 2,
                512..=1023 => 3, 1024..=2047 => 4, 2048..=4095 => 5,
                s if s < 128 => 6,
                _ => 7,
            };
            BUCK[b].fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) } }
}
#[global_allocator]
static A: Counting = Counting;
const IDS:&[&str]=&["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let cap=8usize<<20;
    let mut zs=vec![];
    for id in IDS{
        if let Some(f)=load(id){
            let s=&f[..f.len().min(cap)];
            zs.push((rusty_zstd::compress(s,lvl).unwrap(), s.len()));
        }
    }
    ON.store(1, Ordering::Relaxed);
    for (z,n) in &zs { assert_eq!(rusty_zstd::decompress(z).unwrap().len(), *n); }
    ON.store(0, Ordering::Relaxed);
    println!("ALL allocations during DECODE @ L{lvl}: {}  ({} bytes)
",
        N.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed));
    println!("| size class | count |");
    println!("| --- | ---: |");
    const L: [&str; 8] = ["128..199","200..299","300..511","512..1023",
        "1024..2047","2048..4095","<128","large >=4096"];
    for b in 0..8 {
        let c = BUCK[b].load(Ordering::Relaxed);
        if c == 0 { continue }
        println!("| {} | {c} |", L[b]);
    }
}
