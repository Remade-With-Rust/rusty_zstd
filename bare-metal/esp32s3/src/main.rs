//! rusty_zstd on an ESP32-S3, `no_std + alloc`, with the allocator as an arm.
//!
//! The point of this firmware is one line of output: a round trip that happened
//! on a part with no 64-bit atomics. Until this ran, "bare metal" meant "the
//! compiler accepted it".
//!
//! Xtensa LX7 is 32-bit, so `core::sync::atomic::AtomicU64` does not exist and
//! every census counter in the codec is the `census64` stub. `CENSUS_LIVE` is
//! printed so the log says which build this was.
//!
//! TWO ARMS, one source, selected by a cargo feature:
//!
//! - `arm-esp-alloc` (default): a linked-list heap. Floor is bytes live plus a
//!   header per allocation.
//! - `arm-rusty-alloc`: the house allocator, a size-class page allocator. Floor
//!   is (classes touched) x (page size), independent of bytes requested, so it
//!   is roughly fixed and amortises as the working set grows.
//!
//! `HEAP_BYTES` is patched by `heapsweep.py` to find the smallest heap that
//! still round-trips, which is the number a firmware author actually budgets.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use esp_backtrace as _;
use esp_println::println;

// The image header espflash refuses to flash without.
esp_bootloader_esp_idf::esp_app_desc!();

/// Patched by the sweep script. Both arms are given the same budget.
/// `LEVELS` is patched too, so each level's own heap requirement is readable
/// rather than only the maximum across all three.
const HEAP_BYTES: usize = 192 * 1024;

/// Which levels this build exercises; patched by the sweep script.
const LEVELS: &[i32] = &[1, 3, 5];

#[cfg(all(feature = "arm-esp-alloc", feature = "arm-rusty-alloc"))]
compile_error!("pick ONE allocator arm: two global allocators cannot link");
#[cfg(not(any(feature = "arm-esp-alloc", feature = "arm-rusty-alloc")))]
compile_error!("pick an allocator arm: arm-esp-alloc or arm-rusty-alloc");

// ---------------------------------------------------------------- rusty_alloc
#[cfg(feature = "arm-rusty-alloc")]
use rusty_alloc::prim::fixed::{good_region_size, Region};

/// The region is handed to the backend once. Sized with `good_region_size` so
/// it is whole segments and unpadded: the README warns that a round number
/// strands the remainder, and that wrapping it in an aligned container of your
/// own costs stack.
#[cfg(feature = "arm-rusty-alloc")]
static REGION: Region<{ good_region_size(HEAP_BYTES) }> = Region::new();

#[cfg(feature = "arm-rusty-alloc")]
#[global_allocator]
static ALLOC: rusty_alloc_api::RustyAlloc = rusty_alloc_api::RustyAlloc;

#[cfg(feature = "arm-rusty-alloc")]
const ARM: &str = "rusty_alloc 2.0.5";
#[cfg(feature = "arm-esp-alloc")]
const ARM: &str = "esp-alloc 0.11.0";

/// Compressible but not trivial: repeated phrases with varying joins, so the
/// match finder actually walks chains instead of emitting one long RLE.
fn corpus() -> Vec<u8> {
    let words: [&str; 8] = [
        "the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ", "dog ",
    ];
    let mut v: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    while v.len() < 8 * 1024 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        v.extend_from_slice(words[((x >> 33) as usize) % words.len()].as_bytes());
        // Periodic long repeats, so matches run past the short-match path.
        if (x >> 60) & 7 == 0 {
            let take = core::cmp::min(v.len(), 512);
            let tail: Vec<u8> = v[v.len() - take..].to_vec();
            v.extend_from_slice(&tail);
        }
    }
    v
}

/// Peak bytes the allocator reports having in use, where it can report one.
fn peak_report() {
    #[cfg(feature = "arm-esp-alloc")]
    {
        let s = esp_alloc::HEAP.stats();
        println!(
            "heap  size {}  current {}  PEAK {}",
            s.size, s.current_usage, s.max_usage
        );
    }
    #[cfg(feature = "arm-rusty-alloc")]
    {
        // (elapsed_ms, user_ms, system_ms, current_rss, peak_rss,
        //  current_commit, peak_commit, page_faults); best effort per platform.
        let (_, _, _, cur, peak, ccommit, pcommit, _) = rusty_alloc::stats::process_info();
        println!(
            "heap  current_rss {cur}  PEAK_rss {peak}  commit {ccommit}  peak_commit {pcommit}"
        );
    }
}

#[esp_hal::main]
fn main() -> ! {
    let _p = esp_hal::init(esp_hal::Config::default());

    #[cfg(feature = "arm-esp-alloc")]
    esp_alloc::heap_allocator!(size: HEAP_BYTES);

    #[cfg(feature = "arm-rusty-alloc")]
    let usable = match REGION.give() {
        Ok(u) => u,
        Err(e) => {
            println!("REGION.give FAILED: {e:#x}");
            loop {
                core::hint::spin_loop()
            }
        }
    };

    println!();
    println!("=== rusty_zstd on ESP32-S3 (xtensa, no_std + alloc) ===");
    println!("allocator         {ARM}");
    println!("heap budget       {HEAP_BYTES} bytes");
    #[cfg(feature = "arm-rusty-alloc")]
    println!(
        "region            {} bytes declared, {usable} usable",
        REGION.len()
    );
    println!(
        "census64::CENSUS_LIVE = {}  (false is expected here: no 64-bit atomics)",
        rusty_zstd::census64::CENSUS_LIVE
    );

    let src = corpus();
    println!("source            {} bytes", src.len());

    let mut all_ok = true;
    for level in LEVELS {
        let level = *level;
        match rusty_zstd::compress(&src, level) {
            Ok(z) => {
                let ratio = src.len() as f32 / z.len() as f32;
                #[cfg(feature = "arm-esp-alloc")]
                println!(
                    "   after compress   peak {}  current {}",
                    esp_alloc::HEAP.stats().max_usage,
                    esp_alloc::HEAP.stats().current_usage
                );
                match rusty_zstd::decompress(&z) {
                    Ok(back) => {
                        let ok = back.len() == src.len() && back == src;
                        all_ok &= ok;
                        #[cfg(feature = "arm-esp-alloc")]
                        println!(
                            "   after decompress peak {}  current {}",
                            esp_alloc::HEAP.stats().max_usage,
                            esp_alloc::HEAP.stats().current_usage
                        );
                        println!(
                            "L{level}  {} -> {} bytes  ({:.2}x)  round trip {}",
                            src.len(),
                            z.len(),
                            ratio,
                            if ok { "OK" } else { "MISMATCH" }
                        );
                    }
                    Err(e) => {
                        all_ok = false;
                        println!("L{level}  decompress FAILED: {e:?}");
                    }
                }
            }
            Err(e) => {
                all_ok = false;
                println!("L{level}  compress FAILED: {e:?}");
            }
        }
    }

    println!();
    peak_report();
    println!();
    if all_ok {
        println!("RESULT: PASS -- compressed and decompressed on the board");
    } else {
        println!("RESULT: FAIL");
    }

    loop {
        core::hint::spin_loop()
    }
}
