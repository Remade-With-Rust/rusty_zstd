//! Long-distance matching (`ZSTD_c_enableLongDistanceMatching` / `--long`).

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Default `--long` windowLog (libzstd `ZSTD_WINDOWLOG_LIMIT_DEFAULT` path: 27).
pub const DEFAULT_LONG_WINDOW_LOG: u32 = 27;

/// LDM knobs (`ZSTD_c_ldm*`). Zero means "pick C-like defaults from windowLog".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LdmParams {
    /// Enable the long-distance matcher.
    pub enable: bool,
    /// Hash table log. `0` = `windowLog - 7` (clamped).
    pub hash_log: u32,
    /// Minimum LDM match length. `0` = 64.
    pub min_match: u32,
    /// Insert/lookup every `2^hash_rate_log` bytes. `0` = `windowLog - hashLog`.
    pub hash_rate_log: u32,
}

impl LdmParams {
    /// Enable LDM at `--long[=windowLog]` (default window 27).
    pub fn enabled() -> Self {
        Self {
            enable: true,
            ..Self::default()
        }
    }

    pub(crate) fn resolved(self, window_log: u32) -> ResolvedLdm {
        let min_match = if self.min_match == 0 {
            64
        } else {
            self.min_match.clamp(4, 4096)
        };
        let hash_log = if self.hash_log == 0 {
            window_log.saturating_sub(7).clamp(6, 20)
        } else {
            self.hash_log.clamp(6, 20)
        };
        let hash_rate_log = if self.hash_rate_log == 0 {
            window_log.saturating_sub(hash_log).min(12)
        } else {
            self.hash_rate_log.min(16)
        };
        ResolvedLdm {
            hash_log,
            min_match,
            hash_rate_log,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ResolvedLdm {
    pub hash_log: u32,
    pub min_match: u32,
    pub hash_rate_log: u32,
}

/// Sparse hash of long min-matches.
pub(crate) struct LdmTables {
    hash: Vec<u32>,
}

impl LdmTables {
    pub(crate) fn new(p: ResolvedLdm) -> Self {
        Self {
            hash: vec![0; 1usize << p.hash_log.min(20)],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LdmHit {
    pub ip: usize,
    pub matchlen: u32,
    pub offset: u32,
}

pub(crate) fn prime_ldm(
    tables: &mut LdmTables,
    src: &[u8],
    payload_off: usize,
    window: usize,
    p: ResolvedLdm,
) {
    if payload_off == 0 {
        return;
    }
    let step = (1usize << p.hash_rate_log).max(1);
    let mls = p.min_match as usize;
    let from = payload_off.saturating_sub(window);
    let mut pos = from + (step - from % step) % step;
    while pos + mls <= payload_off && pos + 8 <= src.len() {
        let h = ldm_hash(src, pos, p.hash_log);
        tables.hash[h] = pos as u32;
        pos += step;
    }
}

#[inline(always)]
pub(crate) fn collect_ldm(
    tables: &mut LdmTables,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    p: ResolvedLdm,
    frame_start: usize,
) -> Vec<LdmHit> {
    let step = (1usize << p.hash_rate_log).max(1);
    let mls = p.min_match as usize;
    let mut hits = Vec::new();
    if block_end.saturating_sub(block_start) < mls || src.len() < mls + 8 {
        return hits;
    }
    let ilimit = block_end.saturating_sub(mls);
    let mut ip = block_start;
    let align = (step - (ip % step)) % step;
    ip = ip.saturating_add(align);
    while ip <= ilimit && ip + 8 <= src.len() {
        let h = ldm_hash(src, ip, p.hash_log);
        let m = tables.hash[h] as usize;
        tables.hash[h] = ip as u32;
        if m < ip
            && m >= frame_start
            && ip - m <= window
            && m + mls <= src.len()
            && src[m..m + mls] == src[ip..ip + mls]
        {
            let ml = count_eq(src, m, ip, block_end);
            if ml >= mls {
                let offset = (ip - m) as u32;
                if offset > 0 {
                    hits.push(LdmHit {
                        ip,
                        matchlen: ml as u32,
                        offset,
                    });
                    ip = ip.saturating_add(ml);
                    let align = (step - (ip % step)) % step;
                    ip = ip.saturating_add(align);
                    continue;
                }
            }
        }
        ip = ip.saturating_add(step);
    }
    hits
}

/// Rolling-hash cut for `--rsyncable`. Average gap `2^bits` bytes.
pub(crate) fn rsync_cut(block: &[u8], bits: u32) -> Option<usize> {
    if block.len() < 64 || bits == 0 {
        return None;
    }
    let bits = bits.clamp(6, 20);
    let mask = (1u32 << bits) - 1;
    let mut h = 0u32;
    let last = block.len() - 32;
    for (i, &b) in block.iter().enumerate() {
        h = h
            .rotate_left(1)
            .wrapping_add(u32::from(b).wrapping_mul(0x9E37_79B1));
        if i >= 32 && i < last && (h & mask) == 0 {
            return Some(i + 1);
        }
    }
    None
}

pub(crate) fn rsync_bits(window_log: u32) -> u32 {
    window_log.saturating_sub(10).clamp(8, 16)
}

fn ldm_hash(src: &[u8], ip: usize, hash_log: u32) -> usize {
    // Sixteen individual byte loads assembled into two u64s -- and eight bounds
    // checks with them. Both callers already guard `ip + 8 <= src.len()`
    // explicitly in their loop conditions, and the second word is guarded here,
    // so `load_u64_le` reads each word in ONE unaligned load instead of eight.
    //
    // This is worth doing for the loads, not for the panic count: LDM is
    // `enable_ldm: false` by default, so it is off the shipping path entirely.
    debug_assert!(ip + 8 <= src.len());
    let mut v = crate::simd::load_u64_le(src, ip);
    if ip + 16 <= src.len() {
        let w = crate::simd::load_u64_le(src, ip + 8);
        v ^= w.rotate_left(13);
    }
    let shift = 64u32.saturating_sub(hash_log.min(32));
    (v.wrapping_mul(0xCF1B_BCDC_B7A5_6463) >> shift) as usize
}

fn count_eq(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
    let max = (limit - ip).min(src.len() - m).min(src.len() - ip);
    let mut n = 0usize;
    while n < max && src[m + n] == src[ip + n] {
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{compress_with_advanced, AdvancedOptions};
    use crate::params::compression_params;

    fn aligned_repeat() -> Vec<u8> {
        let mut src = vec![0u8; 4096];
        let mut pat = [0u8; 64];
        for (i, b) in pat.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(1);
        }
        src[0..64].copy_from_slice(&pat);
        src[2048..2112].copy_from_slice(&pat);
        src
    }

    #[test]
    fn collect_ldm_finds_aligned_repeat() {
        let src = aligned_repeat();
        let p = LdmParams::enabled().resolved(18);
        let mut t = LdmTables::new(p);
        let hits = collect_ldm(&mut t, &src, 0, src.len(), 1usize << 18, p, 0);
        assert!(
            hits.iter()
                .any(|h| h.ip == 2048 && h.offset == 2048 && h.matchlen >= 64),
            "hits={hits:?}"
        );
    }

    #[test]
    fn ldm_roundtrip() {
        let src = aligned_repeat().repeat(64);
        let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
        params.window_log = 18;
        let zst = compress_with_advanced(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                ldm: LdmParams::enabled(),
                ..AdvancedOptions::default()
            },
        )
        .expect("ldm compress");
        assert_eq!(crate::decompress(&zst).expect("ldm decode"), src);
    }

    #[test]
    fn rsync_cut_hits_mask() {
        let mut block = vec![0u8; 4096];
        for (i, b) in block.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(31).wrapping_add(7);
        }
        let cut = rsync_cut(&block, 8).expect("rsync cut on 4 KiB");
        assert!(cut > 32 && cut < block.len() - 32, "cut={cut}");
        assert_eq!(rsync_bits(18), 8);
        assert_eq!(rsync_bits(27), 16);
    }
}
