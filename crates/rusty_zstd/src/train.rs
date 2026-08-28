//! Dictionary trainer: COVER, fastcover (CLI default), and legacy.

use crate::dict::{self, public_dict_id, Dictionary};
use crate::encode::harvest_dict_entropy;
use crate::error::Error;
use crate::huffman;
use alloc::vec::Vec;

/// Default `--maxdict` (110 KiB), matching libzstd.
pub const DEFAULT_MAX_DICT: usize = 112_640;

/// Trainer algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainAlgo {
    /// libzstd `--train` / `--train-fastcover`.
    FastCover,
    /// libzstd `--train-cover`.
    Cover,
    /// libzstd `--train-legacy`.
    Legacy,
}

/// Knobs for [`train`].
#[derive(Debug, Clone, Copy)]
pub struct TrainOptions {
    /// Algorithm (CLI `--train` is [`TrainAlgo::FastCover`]).
    pub algo: TrainAlgo,
    /// Maximum dictionary size in bytes (header + content).
    pub max_dict: usize,
    /// Forced Dictionary_ID; `None` picks a public ID from content.
    pub dict_id: Option<u32>,
    /// Segment size. `0` means default (200, or `8 * d`).
    pub k: u32,
    /// d-mer size (6..=16). Default 8.
    pub d: u32,
    /// How many k candidates to try when `k == 0`. Default 4.
    pub steps: u32,
    /// fastcover frequency-table log (10..=31). Default 20.
    pub f: u32,
    /// fastcover acceleration (>= 1). Default 1.
    pub accel: u32,
    /// Fraction of samples used for training (0..=1). Default 1.0.
    pub split: f64,
    /// Legacy selectivity (1..=9). Default 9.
    pub selectivity: u32,
}

impl Default for TrainOptions {
    fn default() -> Self {
        Self::fastcover()
    }
}

impl TrainOptions {
    /// `--train` / `--train-fastcover=d=8,steps=4`.
    pub fn fastcover() -> Self {
        Self {
            algo: TrainAlgo::FastCover,
            max_dict: DEFAULT_MAX_DICT,
            dict_id: None,
            k: 0,
            d: 8,
            steps: 4,
            f: 20,
            accel: 1,
            split: 1.0,
            selectivity: 9,
        }
    }

    /// `--train-cover`.
    pub fn cover() -> Self {
        Self {
            algo: TrainAlgo::Cover,
            ..Self::fastcover()
        }
    }

    /// `--train-legacy`.
    pub fn legacy() -> Self {
        Self {
            algo: TrainAlgo::Legacy,
            ..Self::fastcover()
        }
    }
}

/// Train a zstd dictionary from `samples`. Output is a trained dict C can load.
pub fn train(samples: &[&[u8]], opts: TrainOptions) -> Result<Vec<u8>, Error> {
    if samples.is_empty() || samples.iter().all(|s| s.is_empty()) {
        return Err(Error::Corruption);
    }
    let d = opts.d.clamp(6, 16) as usize;
    let f = opts.f.clamp(10, 24);
    let accel = opts.accel.max(1) as usize;
    let max_dict = opts.max_dict.max(256);
    let split = if opts.split <= 0.0 || opts.split > 1.0 {
        1.0
    } else {
        opts.split
    };
    let n_train = ((samples.len() as f64) * split).ceil().max(1.0) as usize;
    let n_train = n_train.min(samples.len());
    let train_samples = &samples[..n_train];

    let k_list = k_candidates(opts.k, d, opts.steps.max(1), max_dict);
    let mut best_content: Vec<u8> = Vec::new();
    for &k in &k_list {
        let content = match opts.algo {
            TrainAlgo::FastCover => select_fastcover(train_samples, k, d, f, accel, max_dict),
            TrainAlgo::Cover => select_cover(train_samples, k, d, max_dict),
            TrainAlgo::Legacy => select_legacy(train_samples, k, opts.selectivity.max(1), max_dict),
        };
        if content.len() > best_content.len() {
            best_content = content;
        }
    }
    if best_content.len() < 8 {
        best_content = fallback_content(train_samples, max_dict);
    }
    finalize_dictionary(&best_content, train_samples, opts.dict_id, max_dict)
}

fn k_candidates(k: u32, d: usize, steps: u32, max_dict: usize) -> Vec<usize> {
    if k != 0 {
        return vec![k.max(d as u32) as usize];
    }
    let k_min = (d * 8).max(16);
    let k_max = (200usize).max(k_min).min(max_dict.max(k_min));
    let steps = steps.max(1) as usize;
    let mut out = Vec::new();
    for i in 0..steps {
        let k = k_min + i * (k_max.saturating_sub(k_min) / steps.max(1));
        let k = k.max(d).min(max_dict.max(d));
        if !out.contains(&k) {
            out.push(k);
        }
    }
    if out.is_empty() {
        out.push(d.max(16).min(max_dict.max(d)));
    }
    out
}

fn fallback_content(samples: &[&[u8]], max_dict: usize) -> Vec<u8> {
    let mut cat = Vec::new();
    for s in samples {
        cat.extend_from_slice(s);
    }
    if cat.len() > max_dict {
        cat[cat.len() - max_dict..].to_vec()
    } else {
        cat
    }
}

fn hash_dmer(src: &[u8], pos: usize, d: usize, f: u32) -> usize {
    // PAD ELIMINATION REFUTED HERE TWICE. Do not try a third variant.
    //
    //   C3  window once as a slice + `iter().take(8)`/`skip(8)`  **+165**, 14 -> 11 pads
    //   C4  window once as a slice + plain `win[i]` indexing      **+303**, 14 -> 14 pads
    //
    // C4 was the "obvious fix" to C3 -- drop the adaptors, keep the subslice,
    // let the bound come from `win.len()` so LLVM can fold the check. It was
    // worse, and it removed no pads at all: the two `src[pos..pos + X]`
    // slicings introduce their own checks, and the `.min()` chain that hides
    // the bound from LLVM is still there, one level up.
    //
    // The class rule this settles: **a fixed-size `try_into` array wins
    // (`parse_seek_table` 12 -> 0 pads, `parse_trained` 17 -> 6); a
    // runtime-length subslice does not, by either spelling.** The array turns a
    // dynamic bound into a static one. A subslice just moves the dynamic bound.
    // Every remaining pad in this crate is the second shape.
    let mut v = 0u64;
    let n = d.min(8).min(src.len().saturating_sub(pos));
    for i in 0..n {
        v |= u64::from(src[pos + i]) << (8 * i);
    }
    if d > 8 {
        let mut acc = 0u64;
        for i in 8..d.min(src.len().saturating_sub(pos)) {
            acc = acc.wrapping_mul(131).wrapping_add(u64::from(src[pos + i]));
        }
        v ^= acc;
    }
    let shift = 64u32.saturating_sub(f);
    (v.wrapping_mul(0xCF1B_BCDC_B7A5_6463) >> shift) as usize
}

fn packed_dmer(src: &[u8], pos: usize, d: usize) -> u64 {
    let mut v = 0u64;
    let n = d.min(8).min(src.len().saturating_sub(pos));
    for i in 0..n {
        v |= u64::from(src[pos + i]) << (8 * i);
    }
    if d > 8 {
        let mut acc = 0u64;
        for i in 8..d.min(src.len().saturating_sub(pos)) {
            acc = acc.wrapping_mul(131).wrapping_add(u64::from(src[pos + i]));
        }
        v ^= acc;
    }
    v
}

fn select_fastcover(
    samples: &[&[u8]],
    k: usize,
    d: usize,
    f: u32,
    accel: usize,
    max_dict: usize,
) -> Vec<u8> {
    let size = 1usize << f.min(24);
    let mut freqs = vec![0u32; size];
    for sample in samples {
        let mut i = 0usize;
        while i + d <= sample.len() {
            if i % accel == 0 {
                let h = hash_dmer(sample, i, d, f) & (size - 1);
                freqs[h] = freqs[h].saturating_add(1);
            }
            i += 1;
        }
    }
    pick_segments(samples, k, d, max_dict, |sample, pos| {
        freqs[hash_dmer(sample, pos, d, f) & (size - 1)] as u64
    })
}

fn select_cover(samples: &[&[u8]], k: usize, d: usize, max_dict: usize) -> Vec<u8> {
    // Per-dmer sample presence (COVER's frequency), packed into a sorted table.
    let mut keys: Vec<(u64, u32)> = Vec::new();
    for sample in samples {
        let mut seen: Vec<u64> = Vec::new();
        let mut i = 0usize;
        while i + d <= sample.len() {
            seen.push(packed_dmer(sample, i, d));
            i += 1;
        }
        seen.sort_unstable();
        seen.dedup();
        for kmer in seen {
            match keys.binary_search_by_key(&kmer, |e| e.0) {
                Ok(idx) => keys[idx].1 = keys[idx].1.saturating_add(1),
                Err(idx) => keys.insert(idx, (kmer, 1)),
            }
        }
    }
    pick_segments(samples, k, d, max_dict, |sample, pos| {
        let key = packed_dmer(sample, pos, d);
        keys.binary_search_by_key(&key, |e| e.0)
            .map(|i| u64::from(keys[i].1))
            .unwrap_or(0)
    })
}

fn select_legacy(samples: &[&[u8]], k: usize, selectivity: u32, max_dict: usize) -> Vec<u8> {
    let d = k.clamp(8, 16);
    select_fastcover(
        samples,
        k.max(d),
        d,
        16,
        selectivity.max(1) as usize,
        max_dict,
    )
}

#[allow(clippy::needless_range_loop)]
fn pick_segments<F>(samples: &[&[u8]], k: usize, d: usize, max_dict: usize, score_at: F) -> Vec<u8>
where
    F: Fn(&[u8], usize) -> u64,
{
    let k = k.max(d);
    let mut segments: Vec<Vec<u8>> = Vec::new();
    let mut remaining = max_dict;
    let mut used: Vec<Vec<bool>> = samples.iter().map(|s| vec![false; s.len()]).collect();
    loop {
        if remaining == 0 {
            break;
        }
        let mut best: Option<(usize, usize, u64)> = None;
        // C10: `used` is a `Vec<Vec<bool>>`, so every `used[si][i]` was TWO
        // bounds checks -- one for the outer row, one for the element -- and
        // `si` is invariant across the whole inner sweep. Borrowing the row
        // once leaves one check per access instead of two, in the trainer's
        // innermost loops. Safe Rust; identical selection.
        for (si, sample) in samples.iter().enumerate() {
            let used_si = &used[si];
            if sample.len() < k {
                if sample.len() >= d {
                    let mut sc = 0u64;
                    let mut n = 0usize;
                    for i in 0..=sample.len() - d {
                        if !used_si[i] {
                            sc = sc.saturating_add(score_at(sample, i));
                            n += 1;
                        }
                    }
                    if n > 0 && sc > best.map(|b| b.2).unwrap_or(0) {
                        best = Some((si, 0, sc));
                    }
                }
                continue;
            }
            for start in 0..=sample.len() - k {
                if used_si[start] {
                    continue;
                }
                let mut sc = 0u64;
                for i in start..=start + k - d {
                    if !used_si[i] {
                        sc = sc.saturating_add(score_at(sample, i));
                    }
                }
                if sc > best.map(|b| b.2).unwrap_or(0) {
                    best = Some((si, start, sc));
                }
            }
        }
        let Some((si, start, score)) = best else {
            break;
        };
        if score == 0 {
            break;
        }
        let sample = samples[si];
        let take = k.min(sample.len() - start).min(remaining);
        if take == 0 {
            break;
        }
        segments.push(sample[start..start + take].to_vec());
        remaining -= take;
        let end = (start + take).min(sample.len());
        for i in start..end {
            used[si][i] = true;
        }
        if segments.len() > 1024 {
            break;
        }
    }
    let mut content = Vec::new();
    for seg in segments.iter().rev() {
        content.extend_from_slice(seg);
    }
    if content.len() > max_dict {
        content[content.len() - max_dict..].to_vec()
    } else {
        content
    }
}

fn finalize_dictionary(
    content: &[u8],
    samples: &[&[u8]],
    forced_id: Option<u32>,
    max_dict: usize,
) -> Result<Vec<u8>, Error> {
    let mut content = content.to_vec();
    if content.len() < 8 {
        content = fallback_content(samples, max_dict.max(8));
    }
    if content.is_empty() {
        return Err(Error::Corruption);
    }
    let harvested = harvest_dict_entropy(&content, samples)?;
    let tree = huffman::write_tree(&harvested.huff)?;
    let header_len =
        8 + tree.len() + harvested.of_nc.len() + harvested.ml_nc.len() + harvested.ll_nc.len() + 12;
    if header_len >= max_dict {
        return Err(Error::Corruption);
    }
    let cap = max_dict - header_len;
    if content.len() > cap {
        content = content[content.len() - cap..].to_vec();
    }
    let clen = content.len() as u32;
    let mut reps = harvested.reps;
    for r in &mut reps {
        if *r == 0 || *r > clen {
            *r = (*r % clen.max(1)).max(1);
        }
    }
    let id = public_dict_id(&content, forced_id);
    let bytes = dict::write_trained_parts(
        id,
        &tree,
        &harvested.of_nc,
        &harvested.ml_nc,
        &harvested.ll_nc,
        reps,
        &content,
    );
    let _ = Dictionary::from_bytes(&bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compress_using_dict, decompress_using_dict, Dictionary};

    fn samples() -> Vec<Vec<u8>> {
        let base = b"the quick brown fox jumps over the lazy dog. rusty_zstd dict train. ";
        (0..8)
            .map(|i| {
                let mut v = base.repeat(4);
                v.push(b'0' + i);
                v
            })
            .collect()
    }

    #[test]
    fn fastcover_roundtrip_our_decoder() {
        let owned = samples();
        let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
        let dict_bytes = train(
            &refs,
            TrainOptions {
                max_dict: 2048,
                ..TrainOptions::fastcover()
            },
        )
        .expect("train");
        let dict = Dictionary::from_bytes(&dict_bytes).expect("parse trained");
        assert_ne!(dict.id(), 0);
        assert!(dict.content().len() >= 8);
        let src = &owned[0];
        let zst = compress_using_dict(src, &dict, 3).expect("compress dict");
        let got = decompress_using_dict(&zst, &dict).expect("decompress dict");
        assert_eq!(got, *src);
    }

    #[test]
    fn cover_and_legacy_parse() {
        let owned = samples();
        let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
        for opts in [TrainOptions::cover(), TrainOptions::legacy()] {
            let bytes = train(
                &refs,
                TrainOptions {
                    max_dict: 1024,
                    k: 32,
                    ..opts
                },
            )
            .expect("train");
            let d = Dictionary::from_bytes(&bytes).expect("parse");
            assert!(d.content().len() >= 8);
        }
    }

    #[test]
    fn raw_dict_roundtrip() {
        let dict = Dictionary::raw(b"the quick brown fox jumps over");
        let src = b"the quick brown fox jumps over the lazy dog";
        let zst = compress_using_dict(src, &dict, 1).expect("c");
        assert_eq!(decompress_using_dict(&zst, &dict).expect("d"), src);
    }

    #[test]
    fn raw_dict_roundtrip_l3_long() {
        let dict = Dictionary::raw(
            b"the quick brown fox jumps over the lazy dog. rusty_zstd raw dict prefix.".to_vec(),
        );
        let src = b"the quick brown fox jumps over the lazy dog. extra payload bytes here.";
        let zst = compress_using_dict(src, &dict, 3).expect("c");
        let got = decompress_using_dict(&zst, &dict).expect("d");
        assert_eq!(got, src);
        match crate::get_frame_header(&zst).expect("hdr") {
            crate::FrameKind::Zstd(h) => {
                assert!(
                    h.window_size >= dict.content().len() as u64,
                    "window {} dict {}",
                    h.window_size,
                    dict.content().len()
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn prefix_roundtrip() {
        let prefix = b"the quick brown fox jumps over";
        let src = b"the quick brown fox jumps over the lazy dog";
        let zst = crate::compress_using_prefix(src, prefix, 1).expect("c");
        assert_eq!(
            crate::decompress_using_prefix(&zst, prefix).expect("d"),
            src
        );
        let hdr = crate::get_frame_header(&zst).expect("hdr");
        match hdr {
            crate::FrameKind::Zstd(h) => assert_eq!(h.dict_id, None),
            other => panic!("{other:?}"),
        }
    }
}
