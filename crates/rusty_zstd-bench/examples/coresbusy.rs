//! CPU-time / wall-time for a plain one-shot compress.
//!
//! `cores_busy` above 1.0 on a single-threaded workload means threads we are
//! not accounting for. The `rzstd-bench` BINARY installs `rzstd-alloc` as its
//! global allocator; examples do not, so this is the control that says whether
//! the extra CPU belongs to the codec or to the allocator.
fn cpu_ms() -> f64 {
    #[cfg(windows)]
    unsafe {
        // GetProcessTimes via std is not exposed; use the process CPU clock
        // through a cheap proxy: sum of thread times is what the harness reads,
        // so approximate with the same quantity the OS reports for the process.
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn GetProcessTimes(h: isize, a: *mut u64, b: *mut u64, k: *mut u64, u: *mut u64) -> i32;
        }
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) != 0 {
            return (k + u) as f64 / 10_000.0; // 100ns units -> ms
        }
        0.0
    }
    #[cfg(not(windows))]
    0.0
}

fn main() {
    let f = std::fs::read("corpora/data/silesia/dickens").expect("corpus");
    let src = &f[..f.len().min(8 << 20)];
    for lvl in [1, 3, 9] {
        let c0 = cpu_ms();
        let t = std::time::Instant::now();
        let z = rusty_zstd::compress(src, lvl).expect("c");
        let wall = t.elapsed().as_secs_f64() * 1000.0;
        let cpu = cpu_ms() - c0;
        assert!(!z.is_empty());
        println!(
            "L{lvl}: wall {wall:8.1} ms  cpu {cpu:8.1} ms  cores_busy {:.2}",
            cpu / wall.max(0.001)
        );
    }
    println!("\n~1.0 = genuinely single-threaded. ~2.0 = a second thread is being charged to us.");
}
