//! rusty_zstd on an ESP32-S3, `no_std + alloc`.
//!
//! The point of this firmware is one line of output: a round trip that
//! happened on a part with no 64-bit atomics. Until this ran, "bare metal"
//! meant "the compiler accepted it".
//!
//! Xtensa LX7 is 32-bit, so `core::sync::atomic::AtomicU64` does not exist and
//! every census counter in the codec is the `census64` stub. `CENSUS_LIVE` is
//! printed so the log says which build this was.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use esp_backtrace as _;

// The image header espflash refuses to flash without.
esp_bootloader_esp_idf::esp_app_desc!();
use esp_println::println;

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

#[esp_hal::main]
fn main() -> ! {
    let _p = esp_hal::init(esp_hal::Config::default());
    esp_alloc::heap_allocator!(size: 192 * 1024);

    println!();
    println!("=== rusty_zstd on ESP32-S3 (xtensa, no_std + alloc) ===");
    println!(
        "census64::CENSUS_LIVE = {}  (false is expected here: no 64-bit atomics)",
        rusty_zstd::census64::CENSUS_LIVE
    );

    let src = corpus();
    println!("source            {} bytes", src.len());

    let mut all_ok = true;
    for level in [1i32, 3, 5] {
        match rusty_zstd::compress(&src, level) {
            Ok(z) => {
                let ratio = src.len() as f32 / z.len() as f32;
                match rusty_zstd::decompress(&z) {
                    Ok(back) => {
                        let ok = back.len() == src.len() && back == src;
                        all_ok &= ok;
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
    if all_ok {
        println!("RESULT: PASS -- compressed and decompressed on the board");
    } else {
        println!("RESULT: FAIL");
    }

    loop {
        core::hint::spin_loop();
    }
}
