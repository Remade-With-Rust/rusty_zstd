//! SMALL-MESSAGE / DICTIONARY IDENTITY BOARD -- the byte gate for the one-shot
//! driver's per-call machinery.
//!
//! `bytegate` folds whole corpora through `compress()`: one call per (corpus,
//! level), every call on a fresh table set, never a dictionary. It therefore
//! cannot see anything that happens BETWEEN calls -- a reused or restored
//! table set, a digested dictionary, a scratch pool, a cache keyed on the
//! wrong thing. Every one of those is invisible to a per-frame gate and fatal
//! to byte identity, because the failure is "frame N depends on frame N-1".
//!
//! This board is built to make that dependence show:
//!
//!   A  no dictionary: 61 message sizes (0 B .. 64 KiB) x levels 1/3/5/9/15/19
//!      (plus a wider level set at fewer sizes), every round in a different
//!      shuffled order so each (params) follows each other (params), through
//!      every one-shot entry point, with multi-block "dirty" frames (raw,
//!      RLE, repetitive, text) interleaved to leave non-default dispatch
//!      state behind for the next call to inherit if a reset is missing.
//!   B  the same with dictionaries: raw content (112 KiB, 20 KiB, 3 KiB, a
//!      5-byte one that is too short to prime, a 1.5 MiB one that exceeds the
//!      window) and two TRAINED ones, alternated, repeated back to back,
//!      with messages larger than the dictionary, as dictionary and as prefix.
//!   C  the streaming compressor over the same inputs, with and without a
//!      dictionary.
//!   D  multi-threaded jobs (each job is a prefix-primed one-shot).
//!
//! Everything is round-tripped through the matching `decompress*`.
//!
//! A count, not a clock: same number on any machine at any load. Record it
//! before a change, require it after.
//!
//!   cargo run --release -p rusty_zstd-bench --example smallboard
//!
//! `SB_ROUNDS=n` changes the number of shuffled rounds (default 3; the
//! recorded hash is for the default). `SB_VERBOSE=1` prints one line per
//! section-level so a moved hash can be localised.

use rusty_zstd::{
    compress, compress_using_dict, compress_using_dict_with, compress_using_prefix, compress_with,
    compress_with_advanced, compress_with_history, compress_with_params, compression_params,
    decompress, decompress_using_dict, decompress_using_prefix, train, AdvancedOptions,
    CompressOptions, Compressor, Dictionary, Flush, LdmParams, TrainOptions,
};

const LEVELS: &[i32] = &[1, 3, 5, 9, 15, 19];
/// Wider level coverage at fewer sizes: every strategy and the negative ladder.
const XLEVELS: &[i32] = &[-5, -1, 2, 4, 6, 7, 12, 13, 16, 18, 22];
const SIZES: &[usize] = &[
    0, 1, 2, 3, 4, 5, 7, 8, 9, 12, 15, 16, 17, 24, 31, 32, 33, 48, 63, 64, 65, 100, 127, 128, 129,
    200, 255, 256, 257, 300, 511, 512, 513, 700, 1000, 1023, 1024, 1025, 1500, 2047, 2048, 2049,
    3000, 4095, 4096, 4097, 6000, 8191, 8192, 8193, 12000, 16383, 16384, 16385, 20000, 32767,
    32768, 32769, 50000, 65535, 65536,
];
const XSIZES: &[usize] = &[1, 9, 64, 255, 1024, 4097, 16384, 65536];
/// Dictionary section: includes sizes ABOVE every small dictionary.
const DSIZES: &[usize] = &[
    0, 1, 3, 8, 9, 16, 40, 64, 100, 255, 256, 257, 512, 1000, 1024, 1025, 2048, 3000, 4096, 5000,
    8192, 12000, 16384, 20000, 32768, 50000, 65536, 140000, 200000,
];

fn mix(a: &mut u64, b: &[u8]) {
    // Length first, so two outputs cannot trade a byte across their boundary.
    for x in (b.len() as u64).to_le_bytes() {
        *a = (*a ^ u64::from(x)).wrapping_mul(0x100_0000_01B3);
    }
    for &x in b {
        *a = (*a ^ u64::from(x)).wrapping_mul(0x100_0000_01B3);
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % (n.max(1) as u64)) as usize
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

fn load(id: &str, cap: usize) -> Vec<u8> {
    let f = std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .unwrap_or_else(|_| panic!("missing corpus {id} (run from the repo root)"));
    f[..f.len().min(cap)].to_vec()
}

fn cut<'a>(srcs: &'a [Vec<u8>], rng: &mut Rng, which: usize, len: usize) -> &'a [u8] {
    let s = &srcs[which % srcs.len()];
    let len = len.min(s.len());
    let off = rng.below(s.len() - len + 1);
    &s[off..off + len]
}

struct Board {
    gold: u64,
    sect: u64,
    calls: u64,
    bytes_in: u64,
    bytes_out: u64,
    by_level: std::collections::BTreeMap<i32, u64>,
}

impl Board {
    fn new() -> Self {
        Self {
            gold: 0xCBF2_9CE4_8422_2325,
            sect: 0xCBF2_9CE4_8422_2325,
            calls: 0,
            bytes_in: 0,
            bytes_out: 0,
            by_level: Default::default(),
        }
    }
    fn fold(&mut self, level: i32, src: &[u8], z: &[u8]) {
        mix(&mut self.gold, z);
        mix(&mut self.sect, z);
        mix(
            self.by_level.entry(level).or_insert(0xCBF2_9CE4_8422_2325),
            z,
        );
        self.calls += 1;
        self.bytes_in += src.len() as u64;
        self.bytes_out += z.len() as u64;
    }
    fn end_section(&mut self, name: &str, verbose: bool) {
        println!(
            "section {name:<10} calls {:>6}  in {:>10}  out {:>10}  hash {:016X}",
            self.calls, self.bytes_in, self.bytes_out, self.sect
        );
        if verbose {
            for (l, h) in &self.by_level {
                println!("    L{l:<3} {h:016X}");
            }
        }
        self.sect = 0xCBF2_9CE4_8422_2325;
        self.calls = 0;
        self.bytes_in = 0;
        self.bytes_out = 0;
        self.by_level.clear();
    }
}

fn adv_variant(k: usize) -> AdvancedOptions {
    match k % 4 {
        0 => AdvancedOptions::default(),
        1 => AdvancedOptions {
            ldm: LdmParams {
                enable: true,
                ..LdmParams::default()
            },
            ..AdvancedOptions::default()
        },
        2 => AdvancedOptions {
            rsyncable: true,
            ..AdvancedOptions::default()
        },
        _ => AdvancedOptions {
            target_cblock_size: 1300,
            ..AdvancedOptions::default()
        },
    }
}

/// One no-dictionary call through entry point `variant`.
fn plain_call(b: &mut Board, msg: &[u8], level: i32, variant: usize) {
    let n = msg.len() as u64;
    let z = match variant % 6 {
        0 => compress_with(
            msg,
            CompressOptions {
                level,
                checksum: false,
            },
        ),
        1 => compress(msg, level),
        2 => compress_with_params(msg, compression_params(level, Some(n)).unwrap(), true),
        // Parameters that do NOT match the source: a cache keyed on the level
        // or on the length alone would hand this call the wrong tables.
        3 => compress_with_history(
            msg,
            compression_params(level, Some(n * 3 + 5000)).unwrap(),
            false,
            None,
            &[],
            true,
        ),
        4 => compress_with_advanced(
            msg,
            compression_params(level, Some(n)).unwrap(),
            variant % 2 == 0,
            None,
            &[],
            true,
            adv_variant(variant / 6),
        ),
        _ => compress_with(
            msg,
            CompressOptions {
                level,
                checksum: true,
            },
        ),
    }
    .expect("compress");
    assert_eq!(
        &decompress(&z).expect("decompress")[..],
        msg,
        "plain L{level} n={n} v={variant}"
    );
    b.fold(level, msg, &z);
}

/// One dictionary / prefix call through entry point `variant`.
fn dict_call(b: &mut Board, msg: &[u8], dict: &Dictionary, level: i32, variant: usize) {
    let n = msg.len() as u64;
    let dl = dict.content().len() as u64;
    let (z, back) = match variant % 7 {
        0 | 5 => {
            let z = compress_using_dict_with(
                msg,
                dict,
                CompressOptions {
                    level,
                    checksum: variant % 7 == 5,
                },
                variant % 7 == 5,
            )
            .expect("dict compress");
            let d = decompress_using_dict(&z, dict).expect("dict decompress");
            (z, d)
        }
        1 => {
            let z = compress_using_dict(msg, dict, level).expect("dict compress");
            let d = decompress_using_dict(&z, dict).expect("dict decompress");
            (z, d)
        }
        2 => {
            let z = compress_using_prefix(msg, dict.content(), level).expect("prefix compress");
            let d = decompress_using_prefix(&z, dict.content()).expect("prefix decompress");
            (z, d)
        }
        // Parameters sized from the PAYLOAD alone: with a large dictionary the
        // window is then smaller than the dictionary and the tail cut applies.
        3 => {
            let z = compress_with_history(
                msg,
                compression_params(level, Some(n)).unwrap(),
                true,
                Some(dict),
                &[],
                true,
            )
            .expect("history compress");
            let d = decompress_using_dict(&z, dict).expect("dict decompress");
            (z, d)
        }
        4 => {
            let z = compress_with_history(
                msg,
                compression_params(level, Some(n + dl)).unwrap(),
                false,
                None,
                dict.content(),
                false,
            )
            .expect("history compress");
            let d = decompress_using_prefix(&z, dict.content()).expect("prefix decompress");
            (z, d)
        }
        // MT-overlap shape: primed from the prefix, never referencing it.
        _ => {
            let z = compress_with_advanced(
                msg,
                compression_params(level, Some(n + dl)).unwrap(),
                true,
                None,
                dict.content(),
                false,
                AdvancedOptions {
                    prime_only: true,
                    ..AdvancedOptions::default()
                },
            )
            .expect("prime-only compress");
            let d = decompress(&z).expect("prime-only decompress");
            (z, d)
        }
    };
    assert_eq!(&back[..], msg, "dict L{level} n={n} v={variant}");
    b.fold(level, msg, &z);
}

fn stream_call(
    b: &mut Board,
    msg: &[u8],
    dict: Option<&Dictionary>,
    level: i32,
    hint: bool,
    chunk: usize,
) {
    let mut c = Compressor::with_options(
        CompressOptions {
            level,
            checksum: chunk % 2 == 0,
        },
        if hint { Some(msg.len() as u64) } else { None },
    )
    .expect("compressor");
    if let Some(d) = dict {
        c.set_dictionary(d).expect("set_dictionary");
    }
    let mut out = vec![0u8; 1 << 16];
    let mut z = Vec::new();
    for part in msg.chunks(chunk.max(1)) {
        let st = c.stream(part, &mut out, Flush::Continue).expect("stream");
        z.extend_from_slice(&out[..st.output_produced]);
        loop {
            let st = c.stream(&[], &mut out, Flush::Continue).expect("stream");
            z.extend_from_slice(&out[..st.output_produced]);
            if st.output_produced == 0 {
                break;
            }
        }
    }
    loop {
        let st = c.stream(&[], &mut out, Flush::End).expect("stream end");
        z.extend_from_slice(&out[..st.output_produced]);
        if st.done {
            break;
        }
    }
    let back = match dict {
        Some(d) => decompress_using_dict(&z, d).expect("stream dict decompress"),
        None => decompress(&z).expect("stream decompress"),
    };
    assert_eq!(&back[..], msg, "stream L{level} n={}", msg.len());
    b.fold(level, msg, &z);
}

fn main() {
    let rounds: usize = std::env::var("SB_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let verbose = std::env::var("SB_VERBOSE").is_ok();
    let cap = 4 << 20;
    // Text, mixed binary, near-incompressible, structured logs, tiny records,
    // RLE, and true noise: each leaves a different dispatch state behind.
    let ids = [
        "dickens",
        "mozilla",
        "x-ray",
        "jsonlog-16m",
        "smallmsg-8m",
        "zeros-1m",
        "incomp-32m",
    ];
    let srcs: Vec<Vec<u8>> = ids.iter().map(|id| load(id, cap)).collect();
    let dickens_full = load("dickens", usize::MAX);
    let mut b = Board::new();
    let mut rng = Rng(0x5EED_0000_D1C7_B0A2);
    // Progress goes to STDERR with wall time; stdout stays deterministic.
    let t0 = std::time::Instant::now();
    let tick = |what: &str| eprintln!("[{:>7.1}s] {what}", t0.elapsed().as_secs_f64());
    println!(
        "SMALL-MESSAGE / DICTIONARY IDENTITY BOARD  rounds={rounds} sources={}",
        srcs.len()
    );

    // ---- A: no dictionary --------------------------------------------------
    let mut jobs: Vec<(i32, usize)> = Vec::new();
    for &l in LEVELS {
        for &n in SIZES {
            jobs.push((l, n));
        }
    }
    for &l in XLEVELS {
        for &n in XSIZES {
            jobs.push((l, n));
        }
    }
    let dirty_levels = [1, 3, 5, 9, 2, 7, 15, 4];
    for r in 0..rounds {
        tick("A round");
        rng.shuffle(&mut jobs);
        for (i, &(level, n)) in jobs.iter().enumerate() {
            let msg = cut(&srcs, &mut rng, i + r, n);
            plain_call(&mut b, msg, level, i + r);
            if i % 37 == 36 {
                // A multi-block frame that drives the per-frame dispatch state
                // (raw runs, rep yield, step latches, wide chain) off its
                // defaults. The NEXT small call must not see any of it.
                let k = i / 37 + r;
                let dl = dirty_levels[k % dirty_levels.len()];
                let dn = if dl >= 15 { 150_000 } else { 150_000 + rng.below(550_000) };
                let big = cut(&srcs, &mut rng, k, dn);
                plain_call(&mut b, big, dl, k);
            }
        }
    }
    // The same message at the same level, back to back, must be a fixed point.
    for &l in LEVELS {
        let msg = cut(&srcs, &mut rng, 0, 4096);
        let first = compress(msg, l).unwrap();
        for _ in 0..3 {
            assert_eq!(compress(msg, l).unwrap(), first, "repeat L{l}");
        }
        b.fold(l, msg, &first);
    }
    b.end_section("A-plain", verbose);
    tick("A done");

    // ---- B: dictionaries ---------------------------------------------------
    let d0 = (4usize << 20).min(dickens_full.len().saturating_sub(112 * 1024));
    // Explicit `k` and a small sample set: the default parameter search over
    // megabytes of samples is minutes of trainer time this board does not need.
    let samples_small: Vec<&[u8]> = srcs[4].chunks(700).take(120).collect();
    let samples_text: Vec<&[u8]> = dickens_full[..64 << 10].chunks(2048).collect();
    let t_small = train(
        &samples_small,
        TrainOptions {
            max_dict: 6 * 1024,
            k: 64,
            d: 8,
            steps: 1,
            f: 16,
            ..TrainOptions::fastcover()
        },
    )
    .expect("train small");
    tick("trained 1");
    let t_text = train(
        &samples_text,
        TrainOptions {
            max_dict: 12 * 1024,
            k: 256,
            d: 8,
            steps: 1,
            f: 16,
            ..TrainOptions::fastcover()
        },
    )
    .expect("train text");
    tick("trained");
    mix(&mut b.gold, &t_small);
    mix(&mut b.gold, &t_text);
    let dicts: Vec<Dictionary> = vec![
        Dictionary::raw(dickens_full[d0..d0 + 112 * 1024].to_vec()),
        Dictionary::from_bytes(&t_small).expect("parse trained small"),
        Dictionary::raw(srcs[3][100_000..103_000].to_vec()),
        Dictionary::from_bytes(&t_text).expect("parse trained text"),
        Dictionary::raw(b"tiny!".to_vec()),
        Dictionary::raw(dickens_full[1 << 20..(1 << 20) + 20_000].to_vec()),
        Dictionary::raw(srcs[1][..1_500_000].to_vec()),
    ];
    assert!(dicts[1].id() != 0 && dicts[3].id() != 0, "trained dictionaries carry an id");
    // Which source suits which dictionary (so matches into it actually occur),
    // rotated so every dictionary also sees foreign content.
    let home = [0usize, 4, 3, 0, 4, 0, 1];
    let mut djobs: Vec<(i32, usize, usize)> = Vec::new();
    for &l in LEVELS {
        for (k, &n) in DSIZES.iter().enumerate() {
            for d in 0..dicts.len() {
                // Thin the grid: every (level, size) sees three dictionaries.
                if (k + d + l as usize) % 7 < 3 {
                    djobs.push((l, n, d));
                }
            }
        }
    }
    for &l in XLEVELS {
        for (k, &n) in XSIZES.iter().enumerate() {
            djobs.push((l, n, (k + l.unsigned_abs() as usize) % dicts.len()));
        }
    }
    for r in 0..rounds {
        tick("B round");
        rng.shuffle(&mut djobs);
        for (i, &(level, n, d)) in djobs.iter().enumerate() {
            // The 1.5 MiB dictionary at the tree levels is seconds per call;
            // it keeps its coverage at the fast levels.
            let n = if d == 6 && level >= 15 { n.min(20_000) } else { n };
            let which = if (i + r) % 4 == 0 { i } else { home[d] };
            let msg = cut(&srcs, &mut rng, which, n);
            dict_call(&mut b, msg, &dicts[d], level, i + r);
        }
        tick("B grid done");
        // The shape the digested dictionary exists for: ONE dictionary, many
        // small messages back to back; then two dictionaries alternated; then
        // one dictionary at two levels alternated.
        for &(d, l) in &[(0usize, 3i32), (0, 1), (1, 3), (3, 5), (5, 9), (0, 19), (2, 3)] {
            for k in 0..24 {
                let n = [1024usize, 4096, 300, 1024, 9000, 64][k % 6];
                let msg = cut(&srcs, &mut rng, home[d], n);
                dict_call(&mut b, msg, &dicts[d], l, 0);
            }
        }
        for k in 0..48 {
            let d = [0usize, 5][k % 2];
            let msg = cut(&srcs, &mut rng, 0, [1024usize, 2000, 4096][k % 3]);
            dict_call(&mut b, msg, &dicts[d], 3, k % 3);
        }
        for k in 0..48 {
            let d = [1usize, 3, 0][k % 3];
            let l = [3, 1, 5, 3][k % 4];
            let msg = cut(&srcs, &mut rng, home[d], 1024 + k * 37);
            dict_call(&mut b, msg, &dicts[d], l, 0);
        }
        // A clone is the same dictionary; a rebuilt one with the same bytes is
        // too. Neither may behave differently from the original.
        let c = dicts[0].clone();
        let rebuilt = Dictionary::raw(dicts[0].content().to_vec());
        for k in 0..12 {
            let msg = cut(&srcs, &mut rng, 0, 1500);
            let dd = [&dicts[0], &c, &rebuilt][k % 3];
            dict_call(&mut b, msg, dd, 3, 0);
        }
        // Plain calls between dictionary calls at the same level and size.
        for k in 0..24 {
            let msg = cut(&srcs, &mut rng, 0, 2048);
            if k % 2 == 0 {
                dict_call(&mut b, msg, &dicts[0], 3, 0);
            } else {
                plain_call(&mut b, msg, 3, 0);
            }
        }
    }
    b.end_section("B-dict", verbose);
    // REACH: how many of those dictionary calls the digest actually served.
    // Counted, because a digest nothing reaches passes this board untouched.
    #[cfg(feature = "profile")]
    {
        let c = rusty_zstd::take_digest_census();
        println!(
            "    digest reach: at rest {}  re-seated {}  built {}  first sight {}  not eligible {}",
            c[0], c[1], c[2], c[3], c[4]
        );
    }
    tick("B done");

    // ---- C: streaming ------------------------------------------------------
    for r in 0..rounds.min(2) {
        for (i, &l) in [1, 3, 5, 9, 15, 19, 2, 7].iter().enumerate() {
            for (k, &n) in [0usize, 1, 100, 1024, 5000, 40_000, 200_000].iter().enumerate() {
                let n = if l >= 15 { n.min(40_000) } else { n };
                let msg = cut(&srcs, &mut rng, i + k + r, n);
                let d = [None, Some(&dicts[0]), Some(&dicts[1]), Some(&dicts[2])][(i + k + r) % 4];
                stream_call(&mut b, msg, d, l, (i + k) % 2 == 0, [4096, 65_536, 777][(k + r) % 3]);
            }
        }
    }
    b.end_section("C-stream", verbose);
    tick("C done");

    // ---- D: multi-threaded jobs (prefix-primed one-shots) -------------------
    for (i, &l) in [1, 3, 5].iter().enumerate() {
        let msg = cut(&srcs, &mut rng, i, 2_500_000);
        let p = compression_params(l, Some(msg.len() as u64)).unwrap();
        let adv = AdvancedOptions {
            nb_workers: 2,
            job_size: 600_000,
            ..AdvancedOptions::default()
        };
        let z = compress_with_advanced(msg, p, true, None, &[], true, adv).expect("mt");
        assert_eq!(&decompress(&z).expect("mt decompress")[..], msg, "mt L{l}");
        b.fold(l, msg, &z);
        let z = compress_with_advanced(msg, p, false, Some(&dicts[5]), &[], true, adv)
            .expect("mt dict");
        assert_eq!(
            &decompress_using_dict(&z, &dicts[5]).expect("mt dict decompress")[..],
            msg,
            "mt dict L{l}"
        );
        b.fold(l, msg, &z);
    }
    b.end_section("D-mt", verbose);
    tick("D done");

    println!("\nSMALLBOARD {:016X}", b.gold);
}
