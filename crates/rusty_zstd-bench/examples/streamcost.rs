//! STREAMING-DECODER COST CENSUS -- the deterministic receipt for the W4/W5
//! stream bricks (see inline-execution.md section 18).
//!
//! Counts, not clocks: a counting global allocator around the DECODE streaming
//! loop. The old `try_block` did `self.input[a..b].to_vec()` -- one heap
//! allocation plus a full copy of EVERY block payload -- and then
//! `input.drain(..)` after every parsed unit, an O(remaining) memmove per
//! block. Neither is visible to a whole-frame decode; only a chunked feed
//! through `Decompressor::stream` reaches them.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static ON: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, ns: usize) -> *mut u8 {
        if ON.load(Ordering::Relaxed) == 1 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(ns as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(p, l, ns) }
    }
}
#[global_allocator]
static A: C = C;

fn main() {
    let f = std::fs::read("corpora/data/silesia/webster").expect("corpus");
    let src = &f[..f.len().min(32 << 20)];
    let z = rusty_zstd::compress_with(
        src,
        rusty_zstd::CompressOptions { level: 3, checksum: true },
    )
    .unwrap();
    // Our encoder emits 128 KiB blocks at L3, so the block count is exact.
    let blocks = src.len().div_ceil(128 << 10);
    println!("frame {} B, {} blocks, feeding 64 KiB chunks", z.len(), blocks);

    let mut d = rusty_zstd::Decompressor::new();
    let mut out = vec![0u8; rusty_zstd::decompress_stream_out_size()];
    let mut total = 0u64;
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    ON.store(1, Ordering::Relaxed);
    for chunk in z.chunks(64 << 10) {
        let mut fed = false;
        loop {
            let st = d
                .stream(if fed { &[] } else { chunk }, &mut out, false)
                .expect("stream");
            fed = true;
            total += st.output_produced as u64;
            if st.output_produced < out.len() {
                break;
            }
        }
    }
    ON.store(0, Ordering::Relaxed);
    assert_eq!(total as usize, src.len(), "round-trip length");
    println!(
        "chunked decode:  {} allocator calls, {} bytes requested, {:.2} allocs/block",
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
        ALLOCS.load(Ordering::Relaxed) as f64 / blocks as f64
    );
    #[cfg(feature = "profile")]
    {
        let c = rusty_zstd::take_dec_compact();
        println!(
            "chunked compact: {} compactions, {} bytes memmoved ({:.1}x the {} decoded bytes)",
            c[0],
            c[1],
            c[1] as f64 / src.len() as f64,
            src.len()
        );
    }

    // Phase 2 -- the WHOLE frame in one call: does `decoded` stay bounded, or
    // does the progress loop decode all buffered input before returning?
    let mut d = rusty_zstd::Decompressor::new();
    let mut total = 0u64;
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    ON.store(1, Ordering::Relaxed);
    let mut fed = false;
    loop {
        let st = d
            .stream(if fed { &[] } else { &z }, &mut out, true)
            .expect("stream");
        fed = true;
        total += st.output_produced as u64;
        if st.done {
            break;
        }
    }
    ON.store(0, Ordering::Relaxed);
    assert_eq!(total as usize, src.len(), "round-trip length (single-shot)");
    println!(
        "one-shot feed:   {} allocator calls, {} bytes requested",
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    );
    #[cfg(feature = "profile")]
    {
        let c = rusty_zstd::take_dec_compact();
        println!("one-shot compact: {} compactions, {} bytes memmoved", c[0], c[1]);
    }

    // Phase 3 -- STREAMING ENCODE. `emit_block` slides `hist` past the window
    // once it overflows; the question is how often, and what each slide costs
    // (hist memmove + six table fills + a whole-window re-prime).
    let mut c = rusty_zstd::Compressor::new(3).expect("compressor");
    let mut zbuf = vec![0u8; rusty_zstd::compress_stream_out_size()];
    let mut zs: Vec<u8> = Vec::new();
    #[cfg(feature = "profile")]
    let _ = rusty_zstd::take_prime_iters();
    #[cfg(feature = "profile")]
    let _ = rusty_zstd::take_enc_slide();
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    ON.store(1, Ordering::Relaxed);
    let mut chunks = src.chunks(256 << 10).peekable();
    while let Some(chunk) = chunks.next() {
        let flush = if chunks.peek().is_none() {
            rusty_zstd::Flush::End
        } else {
            rusty_zstd::Flush::Continue
        };
        let mut fed = false;
        loop {
            let st = c
                .stream(if fed { &[] } else { chunk }, &mut zbuf, flush)
                .expect("enc stream");
            fed = true;
            zs.extend_from_slice(&zbuf[..st.output_produced]);
            if st.output_produced < zbuf.len() && (flush != rusty_zstd::Flush::End || st.done) {
                break;
            }
            if st.done {
                break;
            }
        }
    }
    ON.store(0, Ordering::Relaxed);
    println!(
        "stream encode:   {} B out, {} allocator calls, {} bytes requested",
        zs.len(),
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    );
    #[cfg(feature = "profile")]
    {
        let s = rusty_zstd::take_enc_slide();
        let p = rusty_zstd::take_prime_iters();
        println!(
            "stream encode:   {} window slides, {} hist bytes memmoved ({:.1}x src), {} prime inserts",
            s[0],
            s[1],
            s[1] as f64 / src.len() as f64,
            p
        );
    }
    assert_eq!(
        rusty_zstd::decompress(&zs).expect("streamed frame decodes"),
        src,
        "stream-encode round-trip"
    );
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, &zs).expect("dump streamed frame");
        println!("streamed frame written to {path}");
    }
}
