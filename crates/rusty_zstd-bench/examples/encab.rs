//! Compress-only in-memory speed, for A/B-ing two BUILDS of the library.
//!
//!   cargo run --release -p rusty_zstd-bench --example encab -- <secs> <levels,csv> <id...>
//!
//! `speedab`'s shape (one program, two library versions, best-of-N in memory,
//! the driver alternates whole processes ABBA on a pinned core) with two
//! differences that matter for an ENCODE campaign: several levels in one
//! process, and no decompress loop -- `speedab` spends half of every budget
//! timing the decoder, which an encoder brick cannot move.
//!
//! Prints one tab-separated row per (id, level):
//!   id  level  src_bytes  compressed_bytes  best_ms  MB/s  loops  best_Mcycles
//! `compressed_bytes` is the work-parity anchor: two arms that claim to be
//! byte-identical must print the same value in every row.
//!
//! `best_Mcycles` is the fastest loop in THREAD CPU cycles
//! (`QueryThreadCycleTime` on Windows; wall nanoseconds elsewhere). A pinned
//! core is not a reserved core: when another tenant runs on it, wall time
//! counts the time this thread spent descheduled and cycle time does not. On
//! 2026-10-04 this box had every P-core at 75-100% from other sessions and the
//! same-binary wall floor moved 50% between two runs minutes apart.

#[global_allocator]
static ALLOC: rzstd_alloc::Alloc = rzstd_alloc::Alloc;

use std::time::{Duration, Instant};

/// CPU cycles this thread has executed (Windows); wall nanoseconds elsewhere.
fn thread_cycles(t0: Instant) -> u64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::GetCurrentThread;
        use windows_sys::Win32::System::WindowsProgramming::QueryThreadCycleTime;
        let mut c = 0u64;
        // SAFETY: the pseudo-handle is always valid for the calling thread and
        // `&mut c` is a writable u64.
        if unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) } != 0 {
            return c;
        }
    }
    t0.elapsed().as_nanos() as u64
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: encab <secs> <levels,csv> <corpus-id...>");
        std::process::exit(2);
    }
    let secs: f64 = a[0].parse().expect("secs");
    let levels: Vec<i32> = a[1].split(',').map(|s| s.parse().expect("level")).collect();
    let budget = Duration::from_secs_f64(secs);
    for id in &a[2..] {
        let mut p = format!("corpora/data/silesia/{id}");
        if !std::path::Path::new(&p).exists() {
            p = format!("corpora/data/generated/{id}");
        }
        let Ok(src) = std::fs::read(&p) else {
            eprintln!("missing {p}");
            continue;
        };
        for &level in &levels {
            // one warm pass outside the timed region
            let warm = rusty_zstd::compress(&src, level).expect("compress");
            let csize = warm.len();
            let mut best = f64::MAX;
            let mut best_cyc = u64::MAX;
            let mut loops = 0u32;
            let t0 = Instant::now();
            loop {
                let t = Instant::now();
                let c0 = thread_cycles(t0);
                let out = rusty_zstd::compress(&src, level).expect("compress");
                let cyc = thread_cycles(t0).saturating_sub(c0);
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                std::hint::black_box(&out);
                if ms < best {
                    best = ms;
                }
                if cyc < best_cyc {
                    best_cyc = cyc;
                }
                loops += 1;
                if t0.elapsed() >= budget && loops >= 5 {
                    break;
                }
            }
            let mb = src.len() as f64 / 1_048_576.0;
            println!(
                "{id}\t{level}\t{}\t{csize}\t{best:.3}\t{:.2}\t{loops}\t{:.3}",
                src.len(),
                mb / (best / 1000.0),
                best_cyc as f64 / 1e6
            );
        }
    }
}
