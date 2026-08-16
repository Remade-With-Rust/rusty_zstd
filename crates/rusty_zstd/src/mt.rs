//! Multi-thread compress: independent frames, optional overlap prefix (`ZSTD_c_nbWorkers`).

use crate::dict::Dictionary;
use crate::encode::{encode_oneshot, AdvancedOptions};
use crate::error::Error;
use crate::params::{CompressionParameters, Strategy};
use alloc::vec::Vec;

/// libzstd `ZSTDMT_NBWORKERS_MAX` on 64-bit.
pub const NB_WORKERS_MAX: u32 = 256;
/// Transparent minimum job size (512 KiB), unless overlap is larger.
pub const JOB_SIZE_MIN: usize = 512 * 1024;

/// Default `--overlap-log` when the caller passes 0 (`zstd` man: 6..=9 by strategy).
pub fn default_overlap_log(strategy: Strategy) -> u32 {
    match strategy {
        Strategy::BtOpt | Strategy::BtUltra | Strategy::BtUltra2 => 9,
        _ => 6,
    }
}

/// Overlap bytes reloaded from the previous job (`overlapLog` 1 = none, 9 = window).
pub fn overlap_size(window_log: u32, overlap_log: u32, strategy: Strategy) -> usize {
    let ov = if overlap_log == 0 {
        default_overlap_log(strategy)
    } else {
        overlap_log.clamp(1, 9)
    };
    if ov <= 1 {
        return 0;
    }
    let window = 1usize << window_log.min(31);
    window >> (9 - ov)
}

/// Job size after C's transparent minimum (`max(512 KiB, overlap, 4*window` if unset)).
pub fn resolve_job_size(requested: usize, window_log: u32, overlap: usize) -> usize {
    let window = 1usize << window_log.min(31);
    let raw = if requested == 0 {
        window.saturating_mul(4).max(1)
    } else {
        requested.max(1)
    };
    raw.max(JOB_SIZE_MIN).max(overlap)
}

/// `std::thread::available_parallelism`, clamped to `1..=NB_WORKERS_MAX` (`-T0`).
pub fn default_nb_workers() -> u32 {
    let n = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    n.clamp(1, NB_WORKERS_MAX as usize) as u32
}

/// Compress `src` as concatenated independent frames, optionally in parallel.
#[allow(clippy::too_many_arguments)]
pub fn compress_mt(
    src: &[u8],
    params: CompressionParameters,
    checksum: bool,
    dict: Option<&Dictionary>,
    prefix: &[u8],
    write_dict_id: bool,
    adv: AdvancedOptions,
) -> Result<Vec<u8>, Error> {
    let workers = adv.nb_workers.clamp(1, NB_WORKERS_MAX) as usize;
    let overlap = overlap_size(params.window_log, adv.overlap_log, params.strategy);
    let job = resolve_job_size(adv.job_size, params.window_log, overlap);
    if src.is_empty() || src.len() <= job {
        let mut one = adv;
        one.nb_workers = 0;
        return encode_oneshot(
            src,
            params,
            checksum,
            Some(src.len() as u64),
            dict,
            prefix,
            write_dict_id,
            one,
        );
    }

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut off = 0usize;
    while off < src.len() {
        let end = (off + job).min(src.len());
        ranges.push((off, end));
        off = end;
    }

    let mut job_adv = adv;
    job_adv.nb_workers = 0;

    let mut parts = Vec::with_capacity(ranges.len());
    parts.resize(ranges.len(), Vec::new());
    run_jobs(
        workers,
        &ranges,
        |_i, (start, end)| {
            let chunk = &src[start..end];
            let ov_prefix: &[u8] = if dict.is_some() {
                &[]
            } else if start == 0 {
                prefix
            } else {
                let ov_from = start.saturating_sub(overlap);
                &src[ov_from..start]
            };
            let mut this_adv = job_adv;
            if start != 0 && dict.is_none() {
                this_adv.prime_only = true;
            }
            encode_oneshot(
                chunk,
                params,
                checksum,
                Some(chunk.len() as u64),
                dict,
                ov_prefix,
                write_dict_id,
                this_adv,
            )
        },
        &mut parts,
    )?;
    let mut out = Vec::new();
    for p in parts {
        out.extend_from_slice(&p);
    }
    Ok(out)
}

fn run_jobs<F>(
    workers: usize,
    ranges: &[(usize, usize)],
    f: F,
    parts: &mut [Vec<u8>],
) -> Result<(), Error>
where
    F: Fn(usize, (usize, usize)) -> Result<Vec<u8>, Error> + Sync,
{
    #[cfg(target_arch = "wasm32")]
    {
        let _ = workers;
        for (i, r) in ranges.iter().copied().enumerate() {
            parts[i] = f(i, r)?;
        }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut i = 0usize;
        while i < ranges.len() {
            let take = (ranges.len() - i).min(workers.max(1));
            let mut err = None;
            std::thread::scope(|s| {
                let mut handles = Vec::with_capacity(take);
                for j in 0..take {
                    let idx = i + j;
                    let r = ranges[idx];
                    let f = &f;
                    handles.push(s.spawn(move || f(idx, r)));
                }
                for (j, h) in handles.into_iter().enumerate() {
                    match h.join() {
                        Ok(Ok(bytes)) => parts[i + j] = bytes,
                        Ok(Err(e)) => err = Some(e),
                        Err(_) => err = Some(Error::Corruption),
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
            i += take;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::AdvancedOptions;
    use crate::inspect_frames;
    use crate::params::compression_params;
    use crate::{decompress, FrameKind};

    fn noise(n: usize) -> Vec<u8> {
        let mut s = 0x4D74_u64;
        let mut v = vec![0u8; n];
        for b in &mut v {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            *b = (s as u8) | 1;
        }
        v
    }

    #[test]
    fn overlap_size_table() {
        assert_eq!(overlap_size(20, 1, Strategy::DFast), 0);
        assert_eq!(overlap_size(20, 9, Strategy::DFast), 1 << 20);
        assert_eq!(overlap_size(20, 8, Strategy::DFast), 1 << 19);
        assert_eq!(overlap_size(20, 6, Strategy::DFast), 1 << 17);
        assert_eq!(default_overlap_log(Strategy::BtUltra2), 9);
        assert_eq!(default_overlap_log(Strategy::DFast), 6);
    }

    #[test]
    fn job_size_enforces_min() {
        let ov = overlap_size(18, 1, Strategy::Fast);
        let j = resolve_job_size(1, 18, ov);
        assert!(j >= JOB_SIZE_MIN);
    }

    #[test]
    fn mt_two_jobs_roundtrip() {
        let src = noise(JOB_SIZE_MIN + JOB_SIZE_MIN / 2);
        let params = compression_params(1, Some(src.len() as u64)).unwrap();
        let zst = compress_mt(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                nb_workers: 2,
                job_size: JOB_SIZE_MIN,
                overlap_log: 1,
                ..AdvancedOptions::default()
            },
        )
        .expect("mt");
        assert_eq!(decompress(&zst).expect("decode"), src);
        let frames = inspect_frames(&zst).expect("list");
        let zstd_n = frames
            .iter()
            .filter(|f| matches!(f.kind, FrameKind::Zstd(_)))
            .count();
        assert!(zstd_n >= 2, "frames={zstd_n}");
    }

    #[test]
    fn mt_overlap_roundtrip() {
        let src = noise(JOB_SIZE_MIN + 64 * 1024);
        let params = compression_params(1, Some(src.len() as u64)).unwrap();
        let zst = compress_mt(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                nb_workers: 2,
                job_size: JOB_SIZE_MIN,
                overlap_log: 9,
                ..AdvancedOptions::default()
            },
        )
        .expect("mt ov");
        assert_eq!(decompress(&zst).expect("decode"), src);
    }

    #[test]
    fn single_job_is_one_frame() {
        let src = noise(1024);
        let params = compression_params(1, Some(src.len() as u64)).unwrap();
        let zst = compress_mt(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                nb_workers: 2,
                job_size: JOB_SIZE_MIN,
                overlap_log: 1,
                ..AdvancedOptions::default()
            },
        )
        .unwrap();
        let frames = inspect_frames(&zst).unwrap();
        assert_eq!(
            frames
                .iter()
                .filter(|f| matches!(f.kind, FrameKind::Zstd(_)))
                .count(),
            1
        );
        assert_eq!(decompress(&zst).unwrap(), src);
    }

    #[test]
    fn mt_overlap_repeating_independent_frames() {
        let src = b"cli completeness rusty_zstd. ".repeat(25_000);
        let params = compression_params(1, Some(src.len() as u64)).unwrap();
        let zst = compress_mt(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                nb_workers: 2,
                job_size: JOB_SIZE_MIN,
                overlap_log: 9,
                ..AdvancedOptions::default()
            },
        )
        .expect("mt ov text");
        assert_eq!(decompress(&zst).expect("decode"), src.as_slice());
        let n = inspect_frames(&zst)
            .unwrap()
            .iter()
            .filter(|f| matches!(f.kind, FrameKind::Zstd(_)))
            .count();
        assert!(n >= 2, "frames={n}");
    }
}
