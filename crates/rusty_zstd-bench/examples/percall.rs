//! PER-CALL FIXED COST of the one-shot entry points, counted not timed.
//!
//! For a small message the encoder's cost is dominated by what it does ONCE
//! PER CALL -- allocations, table zeroing, dictionary copy and priming, knob
//! reads -- and none of that shows on a throughput board taken on megabyte
//! inputs. This counts it, per call, for the shapes the small-message boards
//! time:
//!
//!   allocs/call, bytes requested/call, of which ZEROED (`alloc_zeroed`),
//!   reallocs/call; and with `--features profile`: env reads/call, dictionary
//!   positions primed/call.
//!
//! Deterministic: the same numbers on any machine at any load.
//!
//!   cargo run --release -p rusty_zstd-bench --example percall [--features profile]
//!
//! `PERCALL_SITES=1` adds backtrace attribution of every allocation for the
//! shape named by `PERCALL_SHAPE` (e.g. `plain:1:1024`, `dict:3:1024`); build
//! with `CARGO_PROFILE_RELEASE_DEBUG=1` or the frames will not resolve.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;

static ON: AtomicUsize = AtomicUsize::new(0);
static SITES_ON: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static ZALLOCS: AtomicU64 = AtomicU64::new(0);
static ZBYTES: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static RBYTES: AtomicU64 = AtomicU64::new(0);
static SITES: Mutex<Option<HashMap<String, (u64, u64)>>> = Mutex::new(None);
thread_local! { static REENTRY: Cell<bool> = const { Cell::new(false) }; }

fn site(size: usize, kind: &str) {
    if SITES_ON.load(Relaxed) == 0 {
        return;
    }
    REENTRY.with(|r| {
        if r.get() {
            return;
        }
        r.set(true);
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        // The innermost `rusty_zstd::` frame is the site; keep its caller too,
        // because `Vec::with_capacity` in a helper says little on its own.
        let mut frames: Vec<String> = Vec::new();
        for line in bt.lines() {
            let t = line.trim();
            if let Some(p) = t.find("rusty_zstd::") {
                let s = &t[p..];
                let end = s.find(' ').unwrap_or(s.len());
                let cand = s[..end].trim_end_matches("::{{closure}}");
                if !cand.contains("percall") {
                    frames.push(cand.to_string());
                    if frames.len() == 2 {
                        break;
                    }
                }
            }
        }
        let key = format!("{kind} {}", frames.join(" <- "));
        let mut g = SITES.lock().unwrap();
        let e = g.get_or_insert_with(HashMap::new).entry(key).or_insert((0, 0));
        e.0 += 1;
        e.1 += size as u64;
        r.set(false);
    });
}

struct C;
unsafe impl GlobalAlloc for C {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Relaxed) == 1 {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(l.size() as u64, Relaxed);
            site(l.size(), "alloc  ");
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if ON.load(Relaxed) == 1 {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(l.size() as u64, Relaxed);
            ZALLOCS.fetch_add(1, Relaxed);
            ZBYTES.fetch_add(l.size() as u64, Relaxed);
            site(l.size(), "zeroed ");
        }
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, ns: usize) -> *mut u8 {
        if ON.load(Relaxed) == 1 {
            REALLOCS.fetch_add(1, Relaxed);
            RBYTES.fetch_add(ns as u64, Relaxed);
            site(ns, "realloc");
        }
        unsafe { System.realloc(p, l, ns) }
    }
}
#[global_allocator]
static A: C = C;

fn take() -> [u64; 6] {
    [
        ALLOCS.swap(0, Relaxed),
        BYTES.swap(0, Relaxed),
        ZALLOCS.swap(0, Relaxed),
        ZBYTES.swap(0, Relaxed),
        REALLOCS.swap(0, Relaxed),
        RBYTES.swap(0, Relaxed),
    ]
}

fn run<F: FnMut(&[u8]) -> usize>(name: &str, body: &[u8], n: usize, sites: bool, mut f: F) {
    let msgs: Vec<&[u8]> = body.chunks(n).take(2048).collect();
    // Warm: the first calls legitimately resolve knobs and fill any pool.
    for m in msgs.iter().take(64) {
        f(m);
    }
    #[cfg(feature = "profile")]
    {
        let _ = rusty_zstd::take_env_reads();
        let _ = rusty_zstd::take_prime_iters();
    }
    let _ = take();
    if sites {
        SITES_ON.store(1, Relaxed);
    }
    ON.store(1, Relaxed);
    let mut out = 0usize;
    for m in &msgs {
        out += f(m);
    }
    ON.store(0, Relaxed);
    SITES_ON.store(0, Relaxed);
    let c = take();
    let k = msgs.len() as f64;
    #[cfg(feature = "profile")]
    let extra = format!(
        "  env {:>5.2}  primed {:>9.1}",
        rusty_zstd::take_env_reads() as f64 / k,
        rusty_zstd::take_prime_iters() as f64 / k
    );
    #[cfg(not(feature = "profile"))]
    let extra = String::new();
    println!(
        "{name:<16} calls {:>5}  allocs {:>6.2}  bytes {:>9.0}  zeroed {:>5.2} / {:>9.0} B  reallocs {:>5.2} / {:>8.0} B{extra}  out {out}",
        msgs.len(),
        c[0] as f64 / k,
        c[1] as f64 / k,
        c[2] as f64 / k,
        c[3] as f64 / k,
        c[4] as f64 / k,
        c[5] as f64 / k,
    );
    // Stage split, timed in a SEPARATE pass with the counters off (a count and
    // a clock never share a loop). Profile builds carry the profiler's tax, so
    // this is a decomposition of the call, not a throughput figure.
    #[cfg(feature = "profile")]
    {
        use rusty_zstd::ProfStage as S;
        let mut best = [u64::MAX; 6];
        for _ in 0..5 {
            rusty_zstd::prof_reset();
            let t = std::time::Instant::now();
            for m in &msgs {
                std::hint::black_box(f(m));
            }
            let wall = t.elapsed().as_nanos() as u64;
            let v = [
                wall,
                rusty_zstd::prof_stage_ns(S::EncodeTotal),
                rusty_zstd::prof_stage_ns(S::EncodeTables),
                rusty_zstd::prof_stage_ns(S::EncodeBlocks),
                rusty_zstd::prof_stage_ns(S::EncodeMatchFind),
                rusty_zstd::prof_stage_ns(S::EncodeEntropy),
            ];
            if v[0] < best[0] {
                best = v;
            }
        }
        let c = |x: u64| x as f64 / k;
        println!(
            "    ns/call: wall {:>8.0} = outside {:>6.0} + tables {:>7.0} + setup/prime {:>8.0} + blocks {:>8.0} (find {:>7.0}, entropy {:>7.0}, other {:>6.0})",
            c(best[0]),
            c(best[0].saturating_sub(best[1])),
            c(best[2]),
            c(best[1].saturating_sub(best[2] + best[3])),
            c(best[3]),
            c(best[4]),
            c(best[5]),
            c(best[3].saturating_sub(best[4] + best[5])),
        );
    }
    if sites {
        let mut g = SITES.lock().unwrap();
        if let Some(m) = g.take() {
            let mut v: Vec<_> = m.into_iter().collect();
            v.sort_by_key(|(_, (c, _))| std::cmp::Reverse(*c));
            for (key, (c, b)) in v.iter().take(40) {
                println!("    {:>7.2}/call {:>9.0} B/call  {key}", *c as f64 / k, *b as f64 / k);
            }
        }
    }
}

fn main() {
    let full = std::fs::read("corpora/data/silesia/dickens").expect("corpus (run from the repo root)");
    let body = &full[..4 << 20];
    let dict = rusty_zstd::Dictionary::raw(full[4 << 20..(4 << 20) + 112 * 1024].to_vec());
    let want = std::env::var("PERCALL_SHAPE").unwrap_or_default();
    let sites = std::env::var("PERCALL_SITES").is_ok();
    println!("PER-CALL COST (counts per call; profile={})", cfg!(feature = "profile"));
    for lvl in [1, 3, 5, 9, 19] {
        // 1 and 8 bytes are the FLOOR shapes: too small to hold a match, so
        // the call is the driver plus one tiny block and nothing else.
        for n in [1usize, 8, 64, 256, 1024, 4096, 16384, 65536] {
            let name = format!("plain:{lvl}:{n}");
            if !want.is_empty() && want != name {
                continue;
            }
            if lvl > 3 && n != 1024 && n != 16384 {
                continue;
            }
            run(&name, body, n, sites && !want.is_empty(), |m| {
                rusty_zstd::compress_with(m, rusty_zstd::CompressOptions { level: lvl, checksum: false })
                    .unwrap()
                    .len()
            });
        }
    }
    // A TRAINED dictionary too: it carries entropy tables, and seeding the
    // frame's entropy state from them is per-call work a raw one never does.
    if want.is_empty() || want.starts_with("tdict:") {
        let samples: Vec<&[u8]> = full[..64 << 10].chunks(2048).collect();
        let opts = rusty_zstd::TrainOptions {
            max_dict: 12 * 1024,
            k: 256,
            d: 8,
            steps: 1,
            f: 16,
            ..rusty_zstd::TrainOptions::fastcover()
        };
        let trained = rusty_zstd::Dictionary::from_bytes(&rusty_zstd::train(&samples, opts).unwrap()).unwrap();
        for lvl in [3, 1] {
            let name = format!("tdict:{lvl}:1024");
            if !want.is_empty() && want != name {
                continue;
            }
            run(&name, &body[..2 << 20], 1024, sites && !want.is_empty(), |m| {
                rusty_zstd::compress_using_dict_with(
                    m,
                    &trained,
                    rusty_zstd::CompressOptions { level: lvl, checksum: false },
                    false,
                )
                .unwrap()
                .len()
            });
        }
    }
    for lvl in [3, 1, 5, 9, 19] {
        for n in [1024usize, 4096] {
            let name = format!("dict:{lvl}:{n}");
            if !want.is_empty() && want != name {
                continue;
            }
            run(&name, &body[..2 << 20], n, sites && !want.is_empty(), |m| {
                rusty_zstd::compress_using_dict_with(
                    m,
                    &dict,
                    rusty_zstd::CompressOptions { level: lvl, checksum: false },
                    false,
                )
                .unwrap()
                .len()
            });
        }
    }
}
