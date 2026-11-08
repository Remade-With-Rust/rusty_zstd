//! Dictionary trainer: libzstd's FASTCOVER (CLI default) and COVER, ported
//! from `lib/dictBuilder/{fastcover,cover}.c` (issue #25), plus a legacy
//! entry point.
//!
//! Both build the content the same way. The training samples are one buffer;
//! it is cut into epochs and each epoch gives up one segment of `k` bytes:
//! the window of `k - d + 1` consecutive d-mers whose DISTINCT d-mers have
//! the largest summed frequency. A chosen segment's d-mers have their
//! frequency zeroed, so later epochs look for something else, and segments
//! fill the dictionary from the back, so the first (best) sits at the
//! smallest offsets. FASTCOVER counts hashed d-mers (`2^f` buckets, every
//! occurrence); COVER sorts the d-mers exactly and counts the samples each
//! appears in.
//!
//! Each d-mer is hashed or grouped ONCE, and each epoch is a single sliding
//! pass, so a build is linear in the training size. The previous trainer
//! re-scored every start position of every sample per segment: quadratic,
//! 287 s for 256 KiB of samples where `zstd --train` takes well under 1 s.
//!
//! With `k == 0` or `d == 0` the parameters are searched as libzstd does: k
//! from 50 to 2000 in `steps` steps (and d over {6, 8}), each candidate
//! finalized and scored by the dictionary's size plus the compressed size of
//! the held-out samples at level 3; the smallest total wins, ties to the
//! first. Candidates run on parallel threads; the result does not depend on
//! how many.
//!
//! Content selection matches libzstd 1.5.7 byte for byte for the same
//! samples and parameters (`tests/train_vs_libzstd.rs` style checks live in
//! the bench crate's corpus runs). Two things differ by design:
//! * the entropy tables come from this crate's own harvest, not
//!   `ZDICT_analyzeEntropy`, so a k search can rank close candidates
//!   differently; and
//! * when header + content exceed `max_dict`, the content's FRONT is dropped,
//!   keeping the best segments. `ZDICT_finalizeDictionary` drops the back.

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
    /// libzstd `--train-legacy`: here a single FastCover build (d = 8,
    /// k = 200 unless set), not libzstd's suffix-array legacy trainer.
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
    /// Segment size. `0` (default) searches 50..=2000 in `steps` steps.
    pub k: u32,
    /// d-mer size. FastCover hashes 6 bytes when `d == 6` and 8 otherwise
    /// (libzstd's rule); Cover compares `d` bytes (6..=16). `0` searches
    /// {6, 8}. Default 8.
    pub d: u32,
    /// k candidates when `k == 0`; `0` means 40 (libzstd's library
    /// default). Default 4 (its CLI's).
    pub steps: u32,
    /// FastCover frequency-table log, 10..=24 (libzstd allows up to 31; a
    /// 2^24-entry table is 64 MiB per thread, the cap here). Default 20.
    pub f: u32,
    /// FastCover acceleration, 1..=10: counts every `accel`-th d-mer
    /// (`skip = accel - 1`) and finalizes on a share of the samples, using
    /// libzstd's table. Default 1.
    pub accel: u32,
    /// Fraction of the samples trained on when k or d is searched; the rest
    /// score the candidates. `1.0` trains and scores on all of them; values
    /// outside `(0, 1]` mean `1.0`. FastCover's default is 0.75 and Cover's
    /// 1.0 (the CLI's). With fewer than 5 training samples or no held-out
    /// one, all are used. With k and d both fixed every sample is trained on,
    /// whatever this says, as in libzstd.
    pub split: f64,
    /// Accepted for `--train-legacy=s=#` compatibility; not used.
    pub selectivity: u32,
}

impl Default for TrainOptions {
    fn default() -> Self {
        Self::fastcover()
    }
}

impl TrainOptions {
    /// `--train` / `--train-fastcover`: d = 8, steps = 4, f = 20, split 0.75.
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
            split: 0.75,
            selectivity: 9,
        }
    }

    /// `--train-cover`: d = 8, steps = 4, split 1.0.
    pub fn cover() -> Self {
        Self {
            algo: TrainAlgo::Cover,
            split: 1.0,
            ..Self::fastcover()
        }
    }

    /// `--train-legacy`.
    pub fn legacy() -> Self {
        Self {
            algo: TrainAlgo::Legacy,
            split: 1.0,
            ..Self::fastcover()
        }
    }
}

/// libzstd `FASTCOVER_defaultAccelParameters`: `(finalize %, skip)` by accel.
const ACCEL: [(usize, usize); 11] = [
    (100, 0),
    (100, 0),
    (50, 1),
    (34, 2),
    (25, 3),
    (20, 4),
    (17, 5),
    (14, 6),
    (13, 7),
    (11, 8),
    (10, 9),
];

const PRIME6: u64 = 227_718_039_650_203;
const PRIME8: u64 = 0xCF1B_BCDC_B7A5_6463;

/// Working-memory budget for concurrently running candidates.
const TRAIN_THREAD_BUDGET: usize = 512 << 20;

/// Train a zstd dictionary from `samples`. Output is a trained dict C can load.
pub fn train(samples: &[&[u8]], opts: TrainOptions) -> Result<Vec<u8>, Error> {
    if samples.is_empty() || samples.iter().all(|s| s.is_empty()) {
        return Err(Error::Corruption);
    }
    let max_dict = opts.max_dict.max(256);
    let f = opts.f.clamp(10, 24);
    let accel = (opts.accel.clamp(1, 10)) as usize;
    let (algo, ds, ks, f, accel) = match opts.algo {
        TrainAlgo::Legacy => {
            let k = if opts.k == 0 { 200 } else { opts.k as usize };
            (TrainAlgo::FastCover, alloc::vec![8], alloc::vec![k], 20, 1)
        }
        a => {
            let ds = if opts.d == 0 {
                alloc::vec![6, 8]
            } else {
                alloc::vec![opts.d.clamp(6, 16) as usize]
            };
            let ks = if opts.k == 0 {
                let steps = if opts.steps == 0 {
                    40
                } else {
                    opts.steps as usize
                };
                let step = ((2000 - 50) / steps).max(1);
                (50..=2000).step_by(step).collect()
            } else {
                alloc::vec![opts.k as usize]
            };
            (a, ds, ks, f, accel)
        }
    };
    let search = ds.len() * ks.len() > 1;
    // Without a search there is nothing to hold samples out for: libzstd's
    // fixed-parameter entry points set `splitPoint = 1.0` too.
    let corpus = Corpus::new(samples, if search { opts.split } else { 1.0 });
    let n_finalize = if algo == TrainAlgo::FastCover {
        (corpus.n_train * ACCEL[accel].0 / 100).max(1)
    } else {
        corpus.n_train
    };
    let finalize_set = &samples[..n_finalize];

    let mut best: Option<(usize, Vec<u8>)> = None;
    for &d in &ds {
        // libzstd's parameter check: d <= k <= max_dict. A k given outside
        // it is clamped rather than refused.
        let mut kk: Vec<usize> = ks
            .iter()
            .copied()
            .filter(|&k| k >= d && k <= max_dict)
            .collect();
        if kk.is_empty() {
            kk.push(ks[0].clamp(d, max_dict));
        }
        let engine = Engine::new(algo, &corpus, d, f, ACCEL[accel].1);
        if !search {
            let content = engine.build(&corpus, kk[0], d, max_dict);
            return finalize_dictionary(&content, finalize_set, opts.dict_id, max_dict);
        }
        let cand = Candidates {
            ks: &kk,
            d,
            max_dict,
            finalize_set,
            test: &samples[corpus.test_from..],
            dict_id: opts.dict_id,
        };
        // In `kk` order, and only a strictly smaller total replaces the
        // best: ties go to the first candidate, as `COVER_best_finish` does.
        for (bytes, total) in engine.search(&corpus, &cand).into_iter().flatten() {
            if best.as_ref().is_none_or(|b| total < b.0) {
                best = Some((total, bytes));
            }
        }
    }
    match best {
        Some((_, bytes)) => Ok(bytes),
        None => finalize_dictionary(
            &fallback_content(finalize_set, max_dict),
            finalize_set,
            opts.dict_id,
            max_dict,
        ),
    }
}

/// One k search's inputs.
struct Candidates<'a> {
    ks: &'a [usize],
    d: usize,
    max_dict: usize,
    finalize_set: &'a [&'a [u8]],
    test: &'a [&'a [u8]],
    dict_id: Option<u32>,
}

/// The samples as libzstd's trainers see them: one buffer, sample offsets,
/// and the train/test split.
struct Corpus {
    buf: Vec<u8>,
    /// `offsets[i]..offsets[i + 1]` is sample `i`; `len == n_samples + 1`.
    offsets: Vec<usize>,
    n_train: usize,
    /// First held-out sample (0 when scoring on all of them).
    test_from: usize,
}

impl Corpus {
    fn new(samples: &[&[u8]], split: f64) -> Self {
        let n = samples.len();
        let split = if split > 0.0 && split < 1.0 {
            split
        } else {
            1.0
        };
        // `(unsigned)((double)nbSamples * splitPoint)`, then libzstd's
        // refusal (< 5 training or no test sample) becomes "use them all".
        let (n_train, test_from) = if split < 1.0 {
            let t = ((n as f64) * split) as usize;
            if t >= 5 && t < n {
                (t, t)
            } else {
                (n, 0)
            }
        } else {
            (n, 0)
        };
        let total: usize = samples.iter().map(|s| s.len()).sum();
        let mut buf = Vec::with_capacity(total);
        let mut offsets = Vec::with_capacity(n + 1);
        offsets.push(0);
        for s in samples {
            buf.extend_from_slice(s);
            offsets.push(buf.len());
        }
        Self {
            buf,
            offsets,
            n_train,
            test_from,
        }
    }

    fn train_len(&self) -> usize {
        self.offsets[self.n_train]
    }

    /// d-mer start positions in the training region (`nbDmers`).
    fn n_dmers(&self, d: usize) -> usize {
        (self.train_len() + 1).saturating_sub(d.max(8))
    }
}

fn read_le64(buf: &[u8], pos: usize) -> u64 {
    let w: [u8; 8] = buf[pos..pos + 8].try_into().expect("8 bytes");
    u64::from_le_bytes(w)
}

/// `FASTCOVER_hashPtrToIndex`: `ZSTD_hash6Ptr` for d == 6, else `hash8Ptr`.
fn fc_hash(buf: &[u8], pos: usize, f: u32, d: usize) -> u32 {
    let w = read_le64(buf, pos);
    let h = if d == 6 {
        (w << 16).wrapping_mul(PRIME6)
    } else {
        w.wrapping_mul(PRIME8)
    };
    (h >> (64 - f)) as u32
}

#[derive(Clone, Copy, Default)]
struct Segment {
    begin: usize,
    end: usize,
    score: u64,
}

/// `COVER_computeEpochs`.
fn epochs(max_dict: usize, n_dmers: usize, k: usize, passes: usize) -> (usize, usize) {
    let min_size = k * 10;
    let num = (max_dict / k / passes).max(1);
    let size = n_dmers / num;
    if size >= min_size {
        return (num, size);
    }
    let size = min_size.min(n_dmers);
    (n_dmers / size, size)
}

/// Per-d state shared by every k candidate: each position's d-mer id
/// (FASTCOVER: its hash bucket; COVER: its group) and the id frequencies.
struct Engine {
    algo: TrainAlgo,
    /// `ids[pos]` for every d-mer position in the training region.
    ids: Vec<u32>,
    /// Frequency per id. Candidates work on a copy.
    freqs: Vec<u32>,
}

impl Engine {
    fn new(algo: TrainAlgo, c: &Corpus, d: usize, f: u32, skip: usize) -> Self {
        let n = c.n_dmers(d);
        if n == 0 {
            return Self {
                algo,
                ids: Vec::new(),
                freqs: Vec::new(),
            };
        }
        match algo {
            TrainAlgo::Cover => Self::cover(c, d, n),
            _ => Self::fastcover(c, d, f, skip, n),
        }
    }

    /// `FASTCOVER_ctx_init` + `FASTCOVER_computeFrequency`.
    fn fastcover(c: &Corpus, d: usize, f: u32, skip: usize, n: usize) -> Self {
        let ids: Vec<u32> = (0..n).map(|p| fc_hash(&c.buf, p, f, d)).collect();
        let mut freqs = alloc::vec![0u32; 1usize << f];
        let read = d.max(8);
        for i in 0..c.n_train {
            let mut start = c.offsets[i];
            let end = c.offsets[i + 1];
            while start + read <= end {
                let h = ids[start] as usize;
                freqs[h] = freqs[h].wrapping_add(1);
                start += skip + 1;
            }
        }
        Self {
            algo: TrainAlgo::FastCover,
            ids,
            freqs,
        }
    }

    /// `COVER_ctx_init`: sort the d-mer positions, give each group of equal
    /// d-mers the index of its first sorted entry as id, and count the
    /// DISTINCT samples it occurs in.
    ///
    /// That count is what `COVER_group` means to compute, not what it
    /// computes bit for bit: libzstd sorts with `qsort` and breaks ties by
    /// comparing element ADDRESSES, so the order inside a group (which its
    /// walk assumes ascending) depends on the platform's `qsort`. Exact
    /// COVER parity is not on offer from libzstd itself; FASTCOVER's is.
    fn cover(c: &Corpus, d: usize, n: usize) -> Self {
        let buf = &c.buf;
        let mut suffix: Vec<u32> = (0..n as u32).collect();
        if d <= 8 {
            let mask = if d == 8 {
                u64::MAX
            } else {
                (1u64 << (8 * d)) - 1
            };
            suffix.sort_unstable_by_key(|&p| (read_le64(buf, p as usize) & mask, p));
        } else {
            suffix.sort_unstable_by(|&a, &b| {
                let (a, b) = (a as usize, b as usize);
                buf[a..a + d].cmp(&buf[b..b + d]).then(a.cmp(&b))
            });
        }
        let same = |a: u32, b: u32| -> bool {
            let (a, b) = (a as usize, b as usize);
            buf[a..a + d] == buf[b..b + d]
        };
        let mut ids = alloc::vec![0u32; n];
        let mut freqs = alloc::vec![0u32; n];
        let mut g = 0usize;
        while g < n {
            let mut ge = g + 1;
            while ge < n && same(suffix[g], suffix[ge]) {
                ge += 1;
            }
            let id = g as u32;
            let mut freq = 0u32;
            // Positions within a group ascend (the sort's tie-break), so the
            // sample cursor only moves forward.
            let mut sample_end = 0usize;
            let mut cur = 0usize;
            for &pos in &suffix[g..ge] {
                let pos = pos as usize;
                ids[pos] = id;
                if pos < sample_end {
                    continue;
                }
                freq += 1;
                // First sample end strictly past `pos`.
                cur += c.offsets[cur..].partition_point(|&o| o <= pos);
                sample_end = c.offsets[cur];
            }
            freqs[g] = freq;
            g = ge;
        }
        Self {
            algo: TrainAlgo::Cover,
            ids,
            freqs,
        }
    }

    /// One dictionary's content for this `k` (`*_buildDictionary`).
    fn build(&self, c: &Corpus, k: usize, d: usize, max_dict: usize) -> Vec<u8> {
        let mut freqs = self.freqs.clone();
        let mut active = alloc::vec![0u32; freqs.len()];
        self.build_with(c, k, d, max_dict, &mut freqs, &mut active)
    }

    fn build_with(
        &self,
        c: &Corpus,
        k: usize,
        d: usize,
        max_dict: usize,
        freqs: &mut [u32],
        active: &mut [u32],
    ) -> Vec<u8> {
        let n = self.ids.len();
        if n == 0 || k < d {
            return Vec::new();
        }
        let cover = self.algo == TrainAlgo::Cover;
        let (num, size) = epochs(max_dict, n, k, if cover { 4 } else { 1 });
        let max_zero_run = if cover { (num >> 3).clamp(10, 100) } else { 10 };
        let mut dict = alloc::vec![0u8; max_dict];
        let mut tail = max_dict;
        let mut zero_run = 0usize;
        let mut epoch = 0usize;
        while tail > 0 {
            let begin = epoch * size;
            let seg = self.select(freqs, active, begin, begin + size, k, d, cover);
            epoch = (epoch + 1) % num;
            if seg.score == 0 {
                zero_run += 1;
                if zero_run >= max_zero_run {
                    break;
                }
                continue;
            }
            zero_run = 0;
            let len = (seg.end - seg.begin + d - 1).min(tail);
            if len < d {
                break;
            }
            tail -= len;
            dict[tail..tail + len].copy_from_slice(&c.buf[seg.begin..seg.begin + len]);
        }
        dict.drain(..tail);
        dict
    }

    /// `*_selectSegment`: the best window of `k - d + 1` d-mers in
    /// `begin..end`, scored by the summed frequency of its DISTINCT ids.
    /// `active` counts ids in the window and is left all-zero again.
    #[allow(clippy::too_many_arguments)]
    fn select(
        &self,
        freqs: &mut [u32],
        active: &mut [u32],
        begin: usize,
        end: usize,
        k: usize,
        d: usize,
        cover: bool,
    ) -> Segment {
        let ids = &self.ids;
        let dmers_in_k = k - d + 1;
        let mut best = Segment::default();
        let mut a = Segment {
            begin,
            end: begin,
            score: 0,
        };
        while a.end < end {
            let id = ids[a.end] as usize;
            if active[id] == 0 {
                a.score += u64::from(freqs[id]);
            }
            a.end += 1;
            active[id] += 1;
            if a.end - a.begin == dmers_in_k + 1 {
                let del = ids[a.begin] as usize;
                active[del] -= 1;
                if active[del] == 0 {
                    a.score -= u64::from(freqs[del]);
                }
                a.begin += 1;
            }
            if a.score > best.score {
                best = a;
            }
        }
        while a.begin < end {
            active[ids[a.begin] as usize] -= 1;
            a.begin += 1;
        }
        if cover {
            // COVER trims the zero-frequency head and tail; FASTCOVER does not.
            let mut nb = best.end;
            let mut ne = best.begin;
            for pos in best.begin..best.end {
                if freqs[ids[pos] as usize] != 0 {
                    nb = nb.min(pos);
                    ne = pos + 1;
                }
            }
            best.begin = nb;
            best.end = ne;
        }
        for pos in best.begin..best.end {
            freqs[ids[pos] as usize] = 0;
        }
        best
    }

    /// Build, finalize and score every candidate k: `Some((dict, total))`
    /// per k, in order (`None` where finalizing or scoring failed).
    fn search(&self, c: &Corpus, cand: &Candidates<'_>) -> Vec<Option<(Vec<u8>, usize)>> {
        // Per running candidate: a frequency copy and its window counters
        // (4 + 4 bytes per id), the dictionary buffers, scoring scratch.
        let per_thread = self.freqs.len() * 8 + cand.max_dict * 4 + (4 << 20);
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        let workers = cpus
            .min(cand.ks.len())
            .min((TRAIN_THREAD_BUDGET / per_thread).max(1))
            .max(1);
        let run = |k: usize| -> Option<(Vec<u8>, usize)> {
            let content = self.build(c, k, cand.d, cand.max_dict);
            let bytes =
                finalize_dictionary(&content, cand.finalize_set, cand.dict_id, cand.max_dict)
                    .ok()?;
            let total = score(&bytes, cand.test)?;
            Some((bytes, total))
        };
        let mut out = Vec::with_capacity(cand.ks.len());
        for chunk in cand.ks.chunks(workers) {
            if chunk.len() == 1 {
                out.push(run(chunk[0]));
                continue;
            }
            std::thread::scope(|s| {
                let hs: Vec<_> = chunk.iter().map(|&k| s.spawn(move || run(k))).collect();
                out.extend(hs.into_iter().map(|h| h.join().unwrap_or(None)));
            });
        }
        out
    }
}

/// `COVER_checkTotalCompressedSize`: dictionary size plus every held-out
/// sample compressed with it at level 3 (`ZSTD_CLEVEL_DEFAULT`), no checksum.
fn score(dict_bytes: &[u8], test: &[&[u8]]) -> Option<usize> {
    let dict = Dictionary::from_bytes(dict_bytes).ok()?;
    let opts = crate::CompressOptions {
        level: 3,
        checksum: false,
    };
    let mut total = dict_bytes.len();
    for s in test {
        total += crate::compress_using_dict_with(s, &dict, opts, true)
            .ok()?
            .len();
    }
    Some(total)
}

fn fallback_content(samples: &[&[u8]], max_dict: usize) -> Vec<u8> {
    // Two copies became one. `Vec::new()` grew to the whole sample set by
    // doubling (~1x the samples in realloc traffic), and the tail slice was
    // then copied into a SECOND allocation. The total is known from the slice
    // lengths, and only the last `max_dict` bytes are ever kept -- so reserve
    // exactly what is needed and drop the head in place.
    let total: usize = samples.iter().map(|s| s.len()).sum();
    let mut cat = Vec::with_capacity(
        total
            .min(max_dict.saturating_mul(2))
            .max(total.min(1 << 20)),
    );
    for s in samples {
        cat.extend_from_slice(s);
        // Keeping only the tail bounds the buffer at ~2x `max_dict` instead of
        // the whole sample set, which is the real memory win here.
        if cat.len() > max_dict.saturating_mul(2) {
            let drop = cat.len() - max_dict;
            cat.drain(..drop);
        }
    }
    if cat.len() > max_dict {
        let drop = cat.len() - max_dict;
        cat.drain(..drop);
    }
    cat
}

/// Dictionary positions the entropy harvest may prime in total.
const HARVEST_PRIME_BUDGET: usize = 64 << 20;
/// Samples the harvest always takes when there are that many.
const HARVEST_MIN_SAMPLES: usize = 256;

/// The samples the entropy harvest codes: all of them, or an evenly spaced
/// subset when priming the dictionary once per sample would exceed
/// `HARVEST_PRIME_BUDGET`. The harvest re-primes the whole dictionary for
/// every sample, so 8 MiB of 1 KiB samples against a 110 KiB dictionary was
/// 900 M table inserts -- 3.5 of a build's 3.9 s. The statistics it gathers
/// (literal, LL, ML and offset-code frequencies) settle long before that;
/// libzstd's own `accel` finalizes on as little as 10% of the samples.
fn harvest_subset<'a>(samples: &[&'a [u8]], content_len: usize) -> Vec<&'a [u8]> {
    let cap = (HARVEST_PRIME_BUDGET / content_len.max(1)).max(HARVEST_MIN_SAMPLES);
    if samples.len() <= cap {
        return samples.to_vec();
    }
    (0..cap).map(|i| samples[i * samples.len() / cap]).collect()
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
    let subset = harvest_subset(samples, content.len());
    let harvested = harvest_dict_entropy(&content, &subset)?;
    let tree = huffman::write_tree(&harvested.huff)?;
    let header_len =
        8 + tree.len() + harvested.of_nc.len() + harvested.ml_nc.len() + harvested.ll_nc.len() + 12;
    if header_len >= max_dict {
        return Err(Error::Corruption);
    }
    let cap = max_dict - header_len;
    if content.len() > cap {
        // Was `content[content.len() - cap..].to_vec()` -- a fresh allocation
        // and a full copy to discard a prefix. `drain` moves the tail down in
        // place and keeps the allocation.
        let drop = content.len() - cap;
        content.drain(..drop);
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

    /// 64 samples of 512 bytes of word salad, deterministic. The same
    /// generator wrote the file the pinned libzstd dictionaries below were
    /// trained on (`zstd -B512`, one sample per 512-byte block).
    fn pinned_samples() -> Vec<Vec<u8>> {
        let words: [&[u8]; 12] = [
            b"alpha ",
            b"bravo ",
            b"charlie ",
            b"delta ",
            b"echo ",
            b"foxtrot ",
            b"golf ",
            b"hotel ",
            b"india ",
            b"juliett ",
            b"kilo ",
            b"lima\n",
        ];
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        (0..64)
            .map(|_| {
                let mut v = Vec::with_capacity(520);
                while v.len() < 512 {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    v.extend_from_slice(words[(x % 12) as usize]);
                }
                v.truncate(512);
                v
            })
            .collect()
    }

    /// FASTCOVER content is libzstd 1.5.7's, byte for byte (issue #25). Each
    /// row is `zstd --train-fastcover=k=K,d=D,f=20,split=100,accel=A -B512
    /// --maxdict=112640` on `pinned_samples`: the content length and XXH64 of
    /// the dictionary libzstd wrote. Small enough that neither side trims.
    #[test]
    fn fastcover_content_matches_libzstd() {
        let owned = pinned_samples();
        let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
        for (k, d, accel, len, hash) in [
            (200u32, 8u32, 1u32, 15_878usize, 0x1b84_ce76_330a_8c71u64),
            (100, 6, 3, 2_100, 0x8e18_f335_e863_143e),
            (1000, 8, 1, 25_765, 0x5138_da9a_9e55_019e),
        ] {
            let bytes = train(
                &refs,
                TrainOptions {
                    k,
                    d,
                    accel,
                    split: 1.0,
                    ..TrainOptions::fastcover()
                },
            )
            .expect("train");
            let dict = Dictionary::from_bytes(&bytes).expect("parse");
            let c = dict.content();
            assert_eq!(
                (c.len(), crate::xxh64::xxh64(c)),
                (len, hash),
                "k={k} d={d} accel={accel}"
            );
        }
    }

    /// The k search runs candidates on threads; the result must not depend
    /// on scheduling. Also: a trained dictionary must actually help.
    #[test]
    fn search_is_deterministic_and_helps() {
        let owned = pinned_samples();
        let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
        for opts in [TrainOptions::fastcover(), TrainOptions::cover()] {
            let a = train(&refs, opts).expect("train");
            let b = train(&refs, opts).expect("train");
            assert_eq!(a, b, "{:?}", opts.algo);
            assert!(a.len() <= opts.max_dict);
            let dict = Dictionary::from_bytes(&a).expect("parse");
            let with: usize = refs
                .iter()
                .map(|s| compress_using_dict(s, &dict, 3).expect("c").len())
                .sum();
            let without: usize = refs
                .iter()
                .map(|s| crate::compress(s, 3).expect("c").len())
                .sum();
            assert!(
                with * 4 < without * 3,
                "{:?}: {with} vs {without}",
                opts.algo
            );
        }
    }

    /// `d == 0` searches d too; a tiny `max_dict` clamps k instead of failing.
    #[test]
    fn d_search_and_small_max_dict() {
        let owned = pinned_samples();
        let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
        let bytes = train(
            &refs,
            TrainOptions {
                d: 0,
                steps: 2,
                ..TrainOptions::fastcover()
            },
        )
        .expect("d search");
        Dictionary::from_bytes(&bytes).expect("parse");
        for algo in [TrainOptions::fastcover(), TrainOptions::cover()] {
            let bytes = train(
                &refs,
                TrainOptions {
                    max_dict: 300,
                    k: 5000,
                    ..algo
                },
            )
            .expect("small");
            assert!(bytes.len() <= 300);
        }
    }
}
