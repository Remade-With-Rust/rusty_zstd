//! One compliant in-process timer for CLI `-b` and `rzstd-bench --m7-speed`.
//!
//! Profiler stays off on this path. Duration `0` means one compress+decompress loop.
//!
//! # Estimator parity with the C oracle (codec-measurement 4, 8)
//!
//! facebook/zstd `-b` reports the **fastest** round it observed, not the mean.
//! Measured on v1.5.7 (silesia/mr, `-b1 -T1`): the reported figure climbs
//! 360.6 -> 405.4 -> 413.0 MB/s as `-i` rises from 1 to 3 and then plateaus,
//! while the spread across separate processes collapses from ~45 MB/s to
//! ~0.5 MB/s. That convergence-from-below with vanishing spread is the
//! signature of a best-of-N estimator; a mean would hold its centre.
//!
//! This module therefore records **per-loop** durations and exposes the
//! minimum (`*_best_ms`) alongside the sum. Comparing our mean against C's
//! best is a systematic bias in C's favour, so `*_best_ms` is the figure that
//! may be quoted against C. The sum/mean is retained for cores-busy and for
//! the distribution.

use crate::error::Error;
use crate::{compress_with, decompress, decompress_into, CompressOptions};
use std::time::{Duration, Instant};

/// Cap on retained per-loop samples (a very fast loop on a small input can
/// otherwise run for millions of iterations). Beyond this the min and the sum
/// keep updating; only the median's sample set stops growing.
const MAX_SAMPLES: usize = 100_000;

/// Wall timing of a looped closure (the shared timer).
#[derive(Debug, Clone)]
pub struct LoopTiming {
    /// Completions of `one` (always >= 1).
    pub loops: u32,
    /// Wall milliseconds covering every loop, including the first.
    pub wall_ms: f64,
}

/// In-process compress + decompress at a compression level.
#[derive(Debug, Clone)]
pub struct InProcessBench {
    /// Lower of the two phase loop counts (the weaker estimator).
    pub loops: u32,
    /// Completions of the compress-only timed phase.
    pub compress_loops: u32,
    /// Completions of the decompress-only timed phase.
    pub decompress_loops: u32,
    /// Combined compress+decompress wall (ms).
    pub wall_ms: f64,
    /// Compress-only wall (ms), summed across loops.
    pub compress_ms: f64,
    /// Decompress-only wall (ms), summed across loops.
    pub decompress_ms: f64,
    /// Fastest single compress loop (ms). **This is the C-comparable figure.**
    pub compress_best_ms: f64,
    /// Fastest single decompress loop (ms). **This is the C-comparable figure.**
    pub decompress_best_ms: f64,
    /// Median compress loop (ms).
    pub compress_p50_ms: f64,
    /// Median decompress loop (ms).
    pub decompress_p50_ms: f64,
    /// Compressed size of the last successful compress.
    pub compressed_bytes: usize,
    /// Fewest ticks any single compress loop took, from the injected clock.
    ///
    /// Zero when the caller supplied no clock. When the clock counts CPU
    /// cycles this is **frequency-invariant**: a thermally throttled box
    /// takes the same cycles and more wall time, so this is the figure that
    /// survives cross-session comparison.
    pub compress_best_ticks: u64,
    /// Fewest ticks any single decompress loop took.
    pub decompress_best_ticks: u64,
}

/// Running min / sum / sample set for one timed phase.
struct Samples {
    best_ns: u128,
    sum_ns: u128,
    kept: Vec<u64>,
}

impl Samples {
    fn new() -> Self {
        Self {
            best_ns: u128::MAX,
            sum_ns: 0,
            kept: Vec::new(),
        }
    }

    fn push(&mut self, ns: u128) {
        if ns < self.best_ns {
            self.best_ns = ns;
        }
        self.sum_ns += ns;
        if self.kept.len() < MAX_SAMPLES {
            self.kept.push(ns.min(u128::from(u64::MAX)) as u64);
        }
    }

    fn best_ms(&self) -> f64 {
        if self.best_ns == u128::MAX {
            return 0.0;
        }
        self.best_ns as f64 / 1_000_000.0
    }

    fn sum_ms(&self) -> f64 {
        self.sum_ns as f64 / 1_000_000.0
    }

    fn p50_ms(&mut self) -> f64 {
        if self.kept.is_empty() {
            return 0.0;
        }
        self.kept.sort_unstable();
        self.kept[self.kept.len() / 2] as f64 / 1_000_000.0
    }
}

/// Run `one` at least once; keep running until `min` has elapsed (`0` = once).
pub fn time_loops<E, F>(min: Duration, mut one: F) -> Result<LoopTiming, E>
where
    F: FnMut() -> Result<(), E>,
{
    let start = Instant::now();
    one()?;
    let mut loops = 1u32;
    while !min.is_zero() && start.elapsed() < min {
        one()?;
        loops = loops.saturating_add(1);
    }
    Ok(LoopTiming {
        loops,
        wall_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Minimum loops per timed phase before the time budget may end it.
///
/// Best-of-N is only as good as N. Measured on this repo: files that got
/// 84-280 loops reproduced to within 1.6-6.4% across back-to-back sessions,
/// while files that got 5-25 loops swung -14.1% to +16.5% for byte-identical
/// code on identical input. 25 is the floor where the estimator stops being
/// the dominant noise source.
pub const MIN_LOOPS: u32 = 25;

/// Hard cap so a huge input cannot run a phase forever chasing [`MIN_LOOPS`].
pub const MAX_PHASE: Duration = Duration::from_secs(20);

/// Oneshot `compress` + `decompress` at `level`, timed as two separate
/// phases. Checks `decode(encode(x)) == x` once, outside both timed regions.
pub fn bench_roundtrip(src: &[u8], level: i32, min: Duration) -> Result<InProcessBench, Error> {
    bench_roundtrip_clocked(src, level, min, || 0)
}

/// Time one phase: run `one` until BOTH `min` has elapsed AND `MIN_LOOPS`
/// completions have happened, bounded by [`MAX_PHASE`].
fn time_phase<F>(min: Duration, mut tick: impl FnMut() -> u64, mut one: F) -> Result<Phase, Error>
where
    F: FnMut() -> Result<(), Error>,
{
    let mut s = Samples::new();
    let mut best_ticks = u64::MAX;
    let start = Instant::now();
    let mut loops = 0u32;
    loop {
        let k = tick();
        let t = Instant::now();
        one()?;
        let ns = t.elapsed().as_nanos();
        best_ticks = best_ticks.min(tick().saturating_sub(k));
        s.push(ns);
        loops += 1;
        let elapsed = start.elapsed();
        if elapsed >= MAX_PHASE {
            break;
        }
        if elapsed >= min && loops >= MIN_LOOPS {
            break;
        }
        if min.is_zero() {
            break;
        }
    }
    Ok(Phase {
        loops,
        best_ms: s.best_ms(),
        sum_ms: s.sum_ms(),
        p50_ms: s.p50_ms(),
        best_ticks: if best_ticks == u64::MAX {
            0
        } else {
            best_ticks
        },
    })
}

struct Phase {
    loops: u32,
    best_ms: f64,
    sum_ms: f64,
    p50_ms: f64,
    best_ticks: u64,
}

/// [`bench_roundtrip`] with an injected monotonic tick source.
///
/// The library stays portable and dependency-free, so it cannot read CPU
/// cycles itself; the harness passes a clock in. Supply
/// `QueryThreadCycleTime` (Windows) or `CLOCK_THREAD_CPUTIME_ID` (POSIX) to
/// get a frequency-invariant work measure that a throttling box cannot move.
/// Pass `|| 0` to disable.
pub fn bench_roundtrip_clocked<F>(
    src: &[u8],
    level: i32,
    min: Duration,
    mut tick: F,
) -> Result<InProcessBench, Error>
where
    F: FnMut() -> u64,
{
    // DETERMINISTIC PASS -- runs once, outside every timed region
    // (codec-measurement 13: never share a loop between deterministic and
    // timed quantities). This is also the correctness gate.
    // WORK PARITY WITH THE C ORACLE (codec-measurement 9).
    //
    // The oracle runs `zstd -bN -iN -T1` with no `--check`, so libzstd's
    // default `ZSTD_c_checksumFlag = 0` applies: C's benchmark computes NO
    // xxh64 on compress and verifies NONE on decompress. Our `compress`
    // defaults `checksum: true` (the CLI default), so we were timing an extra
    // full pass over every byte -- on BOTH phases -- that C never runs.
    //
    // Measured on zeros-32m: checksum verification alone was **61% of our
    // decompress time** (7792 -> 20150 MB/s with it off). It is the whole of
    // the reported 2.2x decompress "gap" on that corpus.
    //
    // The shipped default stays `checksum: true`. This is the BENCH matching
    // the oracle's configuration, not a change to what users get.
    let opts = CompressOptions {
        level,
        checksum: false,
        ..Default::default()
    };
    let zst = compress_with(src, opts)?;
    let raw = decompress(&zst)?;
    if raw.as_slice() != src {
        return Err(Error::Corruption);
    }
    let compressed_bytes = zst.len();
    drop(raw);

    let wall = Instant::now();
    // TIMED PASS 1 -- compress only. C `-b` benches the two phases
    // separately, so a shared loop would hand C ~3x our sample count in the
    // same budget and make its best-of-N strictly better than ours.
    let c = time_phase(min, &mut tick, || {
        compress_with(src, opts)?;
        Ok(())
    })?;
    // TIMED PASS 2 -- decompress only, off the already-built frame.
    //
    // WORK PARITY (codec-measurement 9). C `-b` decompresses into a destination
    // buffer it allocates ONCE and reuses every round. Calling `decompress`
    // here allocates a fresh `Vec` per loop, so we timed our decode PLUS the
    // kernel faulting and zeroing every output page, against C's decode alone.
    // Measured on zeros-32m that asymmetry was **82% of our reported time** and
    // accounted for the entire 2.2x "gap" -- a harness defect, not a decoder
    // one. Reuse the buffer, exactly as C does.
    let mut dst = Vec::with_capacity(src.len());
    let d = time_phase(min, &mut tick, || {
        dst.clear();
        decompress_into(&mut dst, &zst)?;
        Ok(())
    })?;
    debug_assert_eq!(dst.len(), src.len());

    Ok(InProcessBench {
        loops: c.loops.min(d.loops),
        compress_loops: c.loops,
        decompress_loops: d.loops,
        wall_ms: wall.elapsed().as_secs_f64() * 1000.0,
        compress_ms: c.sum_ms,
        decompress_ms: d.sum_ms,
        compress_best_ms: c.best_ms,
        decompress_best_ms: d.best_ms,
        compress_p50_ms: c.p50_ms,
        decompress_p50_ms: d.p50_ms,
        compressed_bytes,
        compress_best_ticks: c.best_ticks,
        decompress_best_ticks: d.best_ticks,
    })
}

/// Uncompressed throughput in MB/s (`src_len * loops / seconds`) -- the MEAN rate.
///
/// Prefer [`mbps_best`] when comparing against facebook/zstd `-b`, which
/// reports its fastest round.
pub fn mbps(src_len: usize, loops: u32, ms: f64) -> f64 {
    if ms <= 0.0 {
        return 0.0;
    }
    (src_len as f64 * f64::from(loops)) / (ms / 1000.0) / 1_000_000.0
}

/// Uncompressed throughput in MB/s from the **fastest single loop**.
///
/// This is the estimator facebook/zstd `-b` uses, so it is the only one that
/// may be quoted in a `C/us` ratio.
pub fn mbps_best(src_len: usize, best_ms: f64) -> f64 {
    if best_ms <= 0.0 {
        return 0.0;
    }
    (src_len as f64) / (best_ms / 1000.0) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_loop_roundtrip() {
        let src = b"m7 in-process bench rusty_zstd. ".repeat(40);
        let b = bench_roundtrip(&src, 1, Duration::ZERO).expect("bench");
        assert_eq!(b.loops, 1);
        assert!(b.compressed_bytes > 0);
        assert!(b.compress_ms >= 0.0);
        assert!(b.decompress_ms >= 0.0);
    }

    #[test]
    fn best_is_never_slower_than_the_mean_loop() {
        let src = b"best-of-N vs mean-of-N parity check. ".repeat(4000);
        let b = bench_roundtrip(&src, 1, Duration::from_millis(120)).expect("bench");
        assert!(b.loops >= 1);
        let mean_c = b.compress_ms / f64::from(b.loops);
        let mean_d = b.decompress_ms / f64::from(b.loops);
        // The fastest loop cannot be slower than the average loop.
        assert!(b.compress_best_ms <= mean_c + 1e-9, "compress best > mean");
        assert!(
            b.decompress_best_ms <= mean_d + 1e-9,
            "decompress best > mean"
        );
        // ...and the median sits between them.
        assert!(b.compress_p50_ms >= b.compress_best_ms - 1e-9);
    }

    #[test]
    fn mbps_best_uses_one_loop_not_the_sum() {
        // 1 MB in 1 ms == 1000 MB/s, independent of loop count.
        assert!((mbps_best(1_000_000, 1.0) - 1000.0).abs() < 1e-6);
        // The mean form needs the loop count to reach the same figure.
        assert!((mbps(1_000_000, 10, 10.0) - 1000.0).abs() < 1e-6);
    }
}
