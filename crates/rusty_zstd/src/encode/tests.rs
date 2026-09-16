//! `encode`'s unit tests, split out of `encode.rs`.
//!
//! Moved verbatim: this file is `#[cfg(test)]` only, so a release build
//! never compiles it and the split cannot move the asm board. The parent's
//! private items are in scope through `use super::*` below, which is how a
//! child module reaches them without widening anything to `pub(crate)`.

/// The fixed-width literal push must equal `extend_from_slice` for every
/// run length and every capacity, including the fallback cases (run > 16,
/// no spare capacity, source too close to the end of the input).
#[test]
fn push_literals_matches_extend_from_slice() {
    let src: Vec<u8> = (0..512u32).map(|i| (i % 251) as u8).collect();
    for from in [0usize, 1, 7, 100, 495, 500, 511] {
        for n in 0usize..=40 {
            if from + n > src.len() {
                continue;
            }
            for spare in [0usize, 1, 15, 16, 31, 32, 1024] {
                // GATE 13: every width the dispatch can select, plus the
                // gate-off arm (0). All must equal `extend_from_slice` --
                // the copy writes `w` bytes but publishes only `n`.
                for w in [0usize, super::LIT_PUSH_WIDTH, super::LIT_PUSH_WIDTH_WIDE] {
                    let mut fast = Vec::with_capacity(4 + spare);
                    fast.extend_from_slice(b"HEAD");
                    let mut want = fast.clone();
                    want.extend_from_slice(&src[from..from + n]);
                    super::push_literals(&mut fast, &src, from, from + n, w);
                    assert_eq!(fast, want, "from={from} n={n} spare={spare} w={w}");
                }
            }
        }
    }
}
use super::*;
use crate::{decompress, frame_block_census};

fn rt(src: &[u8], level: i32) {
    let zst = compress(src, level).expect("compress");
    let got = decompress(&zst).unwrap_or_else(|e| {
        panic!(
            "decompress our frame L{level} src={} zst={}: {e:?}",
            src.len(),
            zst.len()
        )
    });
    assert_eq!(got.len(), src.len(), "len level={level}");
    if got != src {
        let pos = got
            .iter()
            .zip(src.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(got.len());
        panic!(
            "mismatch L{level} at {pos}/{} got={:02x} want={:02x} zst={}",
            src.len(),
            got.get(pos).copied().unwrap_or(0),
            src.get(pos).copied().unwrap_or(0),
            zst.len()
        );
    }
}

#[test]
fn silesia_mr_prefix_finder_recon() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("corpora/data/silesia/mr");
    if !path.is_file() {
        return;
    }
    let mut src = std::fs::read(&path).expect("read mr");
    src.truncate(277_521);
    let params = crate::compression_params(1, Some(src.len() as u64)).expect("params");
    let mut tables = MatchTables::new(params);
    let window = 1usize << params.window_log.min(31);
    let mut off = 0usize;
    let mut recon = Vec::new();
    // Mirror `encode_block`: the repeat offsets evolve across blocks, and
    // `find_fast` now SEARCHES them, so a stale [1,4,8] per block would
    // generate different sequences than the real encoder did.
    let mut oracle_reps = [1u32, 4, 8];
    while off < src.len() {
        let end = (off + crate::BLOCKSIZE_MAX as usize).min(src.len());
        let (seqs, lits) = find_fast(&src, off, end, window, params, &mut tables, oracle_reps);
        for sq in &seqs {
            let ov = crate::compressed::offset_value_for(sq.offset, sq.litlen, &oracle_reps);
            let _ = crate::compressed::resolve_offset(ov, sq.litlen, &mut oracle_reps);
        }
        let mut lit_at = 0usize;
        for s in &seqs {
            let n = s.litlen as usize;
            recon.extend_from_slice(&lits[lit_at..lit_at + n]);
            lit_at += n;
            let start = recon
                .len()
                .checked_sub(s.offset as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "offset {} > recon {} off={off} ml={} ll={}",
                        s.offset,
                        recon.len(),
                        s.matchlen,
                        s.litlen
                    )
                });
            for k in 0..s.matchlen as usize {
                recon.push(recon[start + k]);
            }
        }
        recon.extend_from_slice(&lits[lit_at..]);
        off = end;
    }
    assert_eq!(recon.as_slice(), src.as_slice(), "finder recon vs src");
}

/// Split Huffman literals vs FSE sequences on the 277521 `mr` prefix.
#[test]
fn silesia_mr_prefix_entropy_oracle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("corpora/data/silesia/mr");
    if !path.is_file() {
        return;
    }
    let mut src = std::fs::read(&path).expect("read mr");
    src.truncate(277_521);
    let params = crate::compression_params(1, Some(src.len() as u64)).expect("params");
    let mut tables = MatchTables::new(params);
    // Mirror the encode path's table init (encode_frame): `pack_tags` now
    // participates in WIDE dispatch, so an oracle that skips this runs a
    // different finder arm than the frame it is checking against.
    tables.enable_packed_tags(
        params.strategy == Strategy::Fast && tag_alloc_enabled() && fast_pack_enabled(),
        src.len(),
    );
    let window = 1usize << params.window_log.min(31);
    let zst = compress_with(
        &src,
        CompressOptions {
            level: 1,
            checksum: false,
        },
    )
    .expect("nocheck");

    use crate::block::{parse_block_header, BlockType};
    use crate::compressed::BlockState;
    use crate::frame::parse_kind;
    use crate::reader::Reader;
    let mut r = Reader::new(&zst);
    parse_kind(&mut r).expect("frame");
    let mut state = BlockState::new();
    let mut off = 0usize;
    let mut block_i = 0u32;
    let mut decoded = Vec::new();
    let mut oracle_reps = [1u32, 4, 8];
    loop {
        let bh = parse_block_header(&mut r).expect("bh");
        let payload = r.take(bh.payload_len() as usize).expect("payload");
        let end = (off + crate::BLOCKSIZE_MAX as usize).min(src.len());
        let (seqs, lits) = if bh.ty == BlockType::Rle {
            (Vec::new(), Vec::new())
        } else {
            find_fast(&src, off, end, window, params, &mut tables, oracle_reps)
        };
        for sq in &seqs {
            let ov = crate::compressed::offset_value_for(sq.offset, sq.litlen, &oracle_reps);
            let _ = crate::compressed::resolve_offset(ov, sq.litlen, &mut oracle_reps);
        }
        match bh.ty {
            BlockType::Compressed => {
                let mut lr = Reader::new(payload);
                let got_lits = crate::compressed::decode_literals(Vec::new(), &mut lr, &mut state)
                    .unwrap_or_else(|e| panic!("block {block_i} literals: {e:?}"));
                let lit_pos = got_lits
                    .iter()
                    .zip(lits.iter())
                    .position(|(a, b)| a != b)
                    .unwrap_or(got_lits.len().min(lits.len()));
                assert_eq!(
                    got_lits.as_slice(),
                    lits.as_slice(),
                    "block {block_i} Huffman lits mismatch at {lit_pos}/enc={} dec={} nseq={}",
                    lits.len(),
                    got_lits.len(),
                    seqs.len()
                );
                let seq_bytes = lr.take(lr.remaining()).expect("seq bytes");
                let (nseq_d, modes, got_codes) =
                    crate::compressed::debug_seq_codes(seq_bytes, &state)
                        .unwrap_or_else(|e| panic!("block {block_i} seq codes: {e:?}"));
                let mut reps = state.reps;
                let mut want_codes = Vec::new();
                for s in &seqs {
                    let ov = offset_value_for(s.offset, s.litlen, &reps);
                    let _ = resolve_offset(ov, s.litlen, &mut reps).expect("ov");
                    let (llc, _, _) = ll_code(s.litlen, true);
                    let (mlc, _, _) = ml_code(s.matchlen, true);
                    let (ofc, _) = of_code(ov);
                    want_codes.push((s.litlen, s.matchlen, ov, llc, mlc, ofc));
                }
                if got_codes != want_codes {
                    let i = got_codes
                        .iter()
                        .zip(want_codes.iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(got_codes.len().min(want_codes.len()));
                    panic!(
                        "block {block_i} seq codes mismatch at {i}/enc={} dec={} nseq_d={nseq_d} modes={modes:#04x}\n  got={:?}\n want={:?}\n last_got={:?}\n last_want={:?}",
                        want_codes.len(),
                        got_codes.len(),
                        got_codes.get(i),
                        want_codes.get(i),
                        got_codes.last(),
                        want_codes.last()
                    );
                }
                crate::compressed::decode_sequences(
                    seq_bytes,
                    &got_lits,
                    &mut decoded,
                    1u64 << params.window_log.min(31),
                    crate::BLOCKSIZE_MAX,
                    &mut state,
                    &[],
                    0,
                    0,
                )
                .unwrap_or_else(|e| panic!("block {block_i} seqs: {e:?}"));
                let got = &decoded[off..decoded.len().min(end)];
                let want = &src[off..end];
                if got != want {
                    let pos = got
                        .iter()
                        .zip(want.iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(got.len().min(want.len()));
                    panic!(
                        "block {block_i} FSE/exec mismatch at {pos}/{} nseq={} last={:?} trail={}",
                        end - off,
                        seqs.len(),
                        seqs.last(),
                        lits.len() as u32 - seqs.iter().map(|s| s.litlen).sum::<u32>()
                    );
                }
            }
            BlockType::Raw => {
                assert_eq!(payload, &src[off..end], "block {block_i} raw");
                decoded.extend_from_slice(payload);
            }
            BlockType::Rle => {
                decoded.resize(decoded.len() + (end - off), payload[0]);
            }
        }
        off = end;
        block_i += 1;
        if bh.last {
            break;
        }
    }
    assert_eq!(off, src.len());
}

/// Real Silesia `mr` prefix: finder + entropy + us decode.
#[test]
fn silesia_mr_prefix_roundtrip() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("corpora/data/silesia/mr");
    if !path.is_file() {
        return;
    }
    let mut src = std::fs::read(&path).expect("read mr");
    src.truncate(277_521);
    let z_off = compress_with(
        &src,
        CompressOptions {
            level: 1,
            checksum: false,
        },
    )
    .expect("nocheck");
    let got = crate::decompress(&z_off).expect("decompress");
    if got.as_slice() != src.as_slice() {
        let pos = got
            .iter()
            .zip(src.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(got.len().min(src.len()));
        panic!(
            "first mismatch at {pos}/{} got_len={} zst={}",
            src.len(),
            got.len(),
            z_off.len()
        );
    }
}

#[test]
fn silesia_all_oneshot_roundtrip_l1() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("corpora/data/silesia");
    if !dir.is_dir() {
        return;
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("silesia dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "silesia dir exists but has no files: {}",
        dir.display()
    );
    for path in files {
        let src = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let zst =
            compress(&src, 1).unwrap_or_else(|e| panic!("compress {}: {e:?}", path.display()));
        let got = crate::decompress(&zst)
            .unwrap_or_else(|e| panic!("decompress {}: {e:?}", path.display()));
        if got.as_slice() != src.as_slice() {
            let pos = got
                .iter()
                .zip(src.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(src.len()));
            panic!(
                "{} mismatch at {pos}/{} zst={}",
                path.file_name().unwrap().to_string_lossy(),
                src.len(),
                zst.len()
            );
        }
    }
}

/// A HIGHER level must never produce a LARGER file. This FAILED on osdb --
/// L5 (greedy) and L7 (lazy2) both emitted more than L3 (dfast) -- until
/// defects B2 and B3 were fixed. Kept as a standing gate: it is the only
/// thing that catches a match finder silently losing a capability its
/// cheaper neighbour has.
///
///   level  strategy  was         now         C
///   3      dfast     3,613,320   3,613,320   3,501,634
///   5      greedy    3,658,497   3,520,086   3,431,228
///   7      lazy2     3,625,184   3,461,023   3,359,176
///   13     btlazy2   3,370,051   3,152,921
///
/// B2: the lazy look-ahead inserted `ip+1 ..= ip+depth` into the chain, then
/// the back-fill re-inserted them -- `chain[p] = p`, a self-loop the walk
/// breaks on, amputating that hash bucket's whole history (501,705 of them
/// at L7). B3: greedy/lazy/btlazy2 never back-extended a match, a capability
/// `emit_fast_seq` (fast/dfast) has always had and C's lazy has as its
/// "catch up" loop.
/// L3->L5 EXCEPTION, added when DFast back-extension shipped. `bext` is a
/// pure size win on the DFast ladder (-1.276% at L3), and on some corpora it
/// makes L3 beat L5 outright -- osdb by 0.012%, dickens by 1.020%. That is
/// an inversion caused by the CHEAPER level GAINING a capability, which is
/// the opposite event from the one this gate exists to catch, and suppressing
/// a real improvement to preserve the ladder would be backwards.
///
/// On the FULL osdb this test reads, the margin was +0.074% (3,517,111 at L3
/// against 3,519,696 at L5); the 0.012% figure above is the 8 MiB prefix the
/// boards use. The tolerance below is set from the full-file number.
///
/// WIDENED 2026-09-08, same event again. The next-long offset-trade
/// dispatch (`nl_dispatch` defaulted ON, raised cut 24 -> 48) is another
/// pure size win on the DFast ladder, and it takes L3 from 3,517,111 to
/// **3,514,780** on this file. L5 is **3,519,696 -- unchanged, the exact
/// value recorded above**, which is the proof that Greedy did not move
/// and the inversion is once more the cheaper level GAINING. The margin
/// goes +0.074% -> +0.140%, so the bar goes 0.1% -> 0.2%.
///
/// The gate keeps its teeth: the historical DEFECT inversions on this
/// pair were +1.25% and +0.33%, both still far above 0.2%, and
/// `L5_CEILING` is untouched -- L5 sits at 3,519,696 against a 3,530,000
/// ceiling, so a real Greedy regression still fires it.
///
/// The exception is deliberately narrow: this pair only, and a ceiling on L5
/// itself so the gate keeps its teeth. If Greedy ever loses a capability its
/// size grows, `L5_CEILING` fires, and the exception cannot hide it -- which
/// is exactly how B2 and B3 would still be caught today.
const L5_CEILING: usize = 3_530_000;

#[test]
fn higher_level_never_larger_osdb() {
    let Ok(src) = std::fs::read("../../corpora/data/silesia/osdb") else {
        return; // corpus absent
    };
    let mut prev = usize::MAX;
    let mut prev_lvl = 0;
    for lvl in [1, 3, 5, 7, 9, 13, 16, 19] {
        let n = crate::compress(&src, lvl).unwrap().len();
        // The one adjudicated inversion: L3 -> L5, and only as a near-tie.
        // The bar is 0.2%: the measured tie is +0.140%, and the historical
        // DEFECT inversions on this pair were +1.25% and +0.33% -- both far
        // above it, so the gate still catches every defect it ever caught.
        let tie = prev_lvl == 3 && lvl == 5 && n <= prev + prev / 500;
        assert!(
            n <= prev || tie,
            "level {lvl} emitted {n} bytes, more than the previous level's {prev}"
        );
        if lvl == 5 {
            assert!(
                n <= L5_CEILING,
                "L5 emitted {n} bytes, above the {L5_CEILING} ceiling -- Greedy has \
                 lost a capability; the L3->L5 tie exception must not mask that"
            );
        }
        prev = n;
        prev_lvl = lvl;
    }
}

/// Truth table for the probe-density (pair-search) dispatch: per file, the
/// L1 size delta from `RZSTD_STEP0=1` joined to the finder's own counters
/// measured at the shipping `step0=2`. Needs `--features profile`.
#[ignore]
#[test]
fn probe_density_truth_table() {
    const FILES: &[&str] = &[
        "dickens", "mozilla", "mr", "nci", "ooffice", "osdb", "reymont", "samba", "sao", "webster",
        "x-ray", "xml",
    ];
    println!(
        "TT {:<9} {:>10} {:>9} {:>9} {:>10} {:>9}",
        "file", "probes/B", "hit_rate", "matchfrac", "lit_share", "seqs/B"
    );
    for f in FILES {
        let Ok(src) = std::fs::read(format!("../../corpora/data/silesia/{f}")) else {
            continue;
        };
        crate::prof::reset();
        let n = crate::compress(&src, 1).unwrap().len();
        let c = crate::prof::encode_counts();
        let b = src.len() as f64;
        println!(
            "TT {f:<9} {:>10.4} {:>9.4} {:>9.4} {:>10.4} {:>9.5}  size={n}",
            c.hash_probes as f64 / b,
            c.probe_hits as f64 / (c.hash_probes.max(1)) as f64,
            c.match_bytes as f64 / b,
            c.lit_bytes as f64 / b,
            c.seqs as f64 / b,
        );
    }
}

/// P0/gg-matchfind gate: the deterministic WORK COUNTER must be non-zero at
/// EVERY level, for every strategy. It was reported only by `find_fast` and
/// `find_lazy`, so `work` -- the PRIMARY evidence under the Great Gate
/// 2026-08-06 law -- did not exist for levels 2-22 and no gate on them could
/// be banked. Needs `--features profile`.
#[cfg(feature = "profile")]
#[test]
fn work_counter_covers_every_strategy() {
    let Ok(src) = std::fs::read("../../corpora/data/silesia/xml") else {
        return; // corpus absent
    };
    // one level per strategy: fast, dfast, greedy, lazy, lazy2, btlazy2, btopt, btultra
    for (lvl, strat) in [
        (1, "fast"),
        (3, "dfast"),
        (5, "greedy"),
        (7, "lazy"),
        (9, "lazy2"),
        (13, "btlazy2"),
        (17, "btopt"),
        (19, "btultra"),
    ] {
        crate::prof::reset();
        let _ = crate::compress(&src, lvl).unwrap();
        let c = crate::prof::encode_counts();
        println!(
            "WORK L{lvl:<2} {strat:<8} probes={:<12} hits={:<10} seqs={:<10}",
            c.hash_probes, c.probe_hits, c.seqs
        );
        assert!(
            c.hash_probes > 0,
            "L{lvl} ({strat}) reported ZERO probes -- the work counter is \
             missing for this strategy, so no gate on it is bankable"
        );
        assert!(c.seqs > 0, "L{lvl} ({strat}) reported zero sequences");
    }
}

/// Deterministic compressed-size table over Silesia -- run before and
/// after a size-affecting change and diff the rows.
#[ignore]
#[test]
fn size_table_silesia() {
    const FILES: &[&str] = &[
        "dickens", "mozilla", "mr", "nci", "ooffice", "osdb", "reymont", "samba", "sao", "webster",
        "x-ray", "xml",
    ];
    for f in FILES {
        let Ok(src) = std::fs::read(format!("../../corpora/data/silesia/{f}")) else {
            continue;
        };
        let mut row = format!("{f:<8}");
        for lvl in [5, 7, 9, 13, 19] {
            let n = crate::compress(&src, lvl).unwrap().len();
            row.push_str(&format!(" L{lvl}={n}"));
        }
        println!("SIZETABLE {row}");
    }
}

#[test]
fn census_zeros_all_rle() {
    let src = vec![0u8; 128 * 1024 * 2];
    let zst = compress(&src, 1).expect("compress");
    let c = crate::frame_block_census(&zst).expect("census");
    assert_eq!(c.compressed, 0);
    assert_eq!(c.raw, 0);
    assert_eq!(c.rle, 2);
    assert_eq!(c.rle_regen, src.len() as u64);
}

#[test]
fn frame_checksum_matches_oneshot_xxh64() {
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text = Vec::new();
    while text.len() < 200_000 {
        text.extend_from_slice(fox);
    }
    for src in [&b""[..], b"a", &[0u8; 128 * 1024 + 7][..], text.as_slice()] {
        let zst = compress(src, 1).expect("compress");
        assert!(zst.len() >= 4);
        let got = u32::from_le_bytes(zst[zst.len() - 4..].try_into().unwrap());
        assert_eq!(got, content_checksum(src), "len {}", src.len());
    }
}

/// D8a gate (inline-execution V1). `Xxh64::update` now routes its bulk
/// through the AVX2 hybrid, which consumes whole 256-byte TILES, while the
/// hasher itself buffers on a 32-byte STRIPE boundary. Those two boundaries
/// must agree for every possible phase between them, so the streaming
/// digest is driven at chunk sizes that land on, just under and just over
/// each of them -- and compared against the one-shot, which takes a
/// different route through the same arithmetic.
///
/// This is a FORMAT checksum. A single differing bit is a corrupt frame, so
/// the gate is exhaustive over the boundary phases rather than sampled.
#[test]
fn streaming_xxh64_matches_oneshot_at_every_boundary_phase() {
    let mut data = Vec::new();
    for i in 0usize..(3 * 1024 + 37) {
        data.push((i.wrapping_mul(2654435761) >> 11) as u8);
    }
    // 1/7/31/32/33 straddle the stripe boundary, 127/128/129 the scalar
    // chunk, 255/256/257 the AVX2 tile, 1024 clears several tiles at once.
    const CHUNKS: &[usize] = &[
        1, 7, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256, 257, 511, 512, 513, 1024,
    ];
    for &len in &[
        0usize,
        1,
        31,
        32,
        33,
        255,
        256,
        257,
        511,
        512,
        1000,
        3072,
        3 * 1024 + 37,
    ] {
        let src = &data[..len];
        let want = content_checksum(src);
        for &c in CHUNKS {
            let mut h = Xxh64::new();
            for part in src.chunks(c) {
                h.update(part);
            }
            assert_eq!(
                checksum_u32(&h),
                want,
                "streaming digest diverged: len {len}, chunk {c}"
            );
        }
        // Uneven schedule: a big feed, then a byte, then the rest -- the
        // case a fixed chunk size cannot produce.
        for &split in &[1usize, 31, 32, 33, 255, 256, 257] {
            if split >= len {
                continue;
            }
            let mut h = Xxh64::new();
            h.update(&src[..split]);
            h.update(&src[split..split + 1]);
            h.update(&src[split + 1..]);
            assert_eq!(
                checksum_u32(&h),
                want,
                "streaming digest diverged: len {len}, split {split}"
            );
        }
    }
}

#[test]
fn roundtrip_small_all_fast_levels() {
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text = Vec::new();
    while text.len() < 8192 {
        text.extend_from_slice(fox);
    }
    for level in -7i32..=3 {
        rt(b"", level);
        rt(b"a", level);
        rt(b"hello", level);
        rt(&[0u8; 16], level);
        rt(&[0u8; 256], level);
        rt(&[0u8; 4096], level);
        rt(&text, level);
        rt(&xorshift(0xA5A5_5A5A, 1024), level);
        rt(&xorshift(0xA5A5_5A5A, 64 * 1024), level);
    }
}

#[test]
fn roundtrip_mid_and_high_levels() {
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text = Vec::new();
    while text.len() < 8192 {
        text.extend_from_slice(fox);
    }
    let noise = xorshift(0xA5A5_5A5A, 8192);
    for level in [4, 5, 6, 8, 9, 13, 16, 19] {
        rt(&text, level);
        rt(&noise, level);
        rt(&[0u8; 1024], level);
    }
}

#[test]
fn huffman_literals_emitted_on_text() {
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text = Vec::new();
    while text.len() < 224 {
        text.extend_from_slice(fox);
    }
    text.truncate(224);
    let (sec, upd) = crate::huffman::encode_literals_section(&text, None).unwrap();
    assert_eq!(sec[0] & 3, 2, "expected Huffman Compressed literals");
    match upd {
        crate::huffman::HuffUpdate::New(_) => {}
        crate::huffman::HuffUpdate::Unchanged => panic!("expected a new Huffman table"),
    }
    let zst = compress(&text, 1).unwrap();
    assert_eq!(decompress(&zst).unwrap(), text);
}

#[test]
fn roundtrip_greedy_explicit() {
    let opts = CompressOptions {
        level: 5,
        ..CompressOptions::default()
    };
    let src = xorshift(0x1111_2222, 32 * 1024);
    let zst = compress_with(&src, opts).unwrap();
    assert_eq!(decompress(&zst).unwrap(), src);
}

#[test]
fn zeros_and_text_shrink() {
    let zeros = vec![0u8; 4096];
    let zst = compress(&zeros, 1).unwrap();
    assert!(
        zst.len() < zeros.len(),
        "zeros L1 {} vs {}",
        zst.len(),
        zeros.len()
    );
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(64);
    let zst = compress(&fox, 3).unwrap();
    assert!(
        zst.len() < fox.len(),
        "text L3 {} vs {}",
        zst.len(),
        fox.len()
    );
}

#[test]
fn rle_byte_word_matches_byte_all() {
    assert_eq!(rle_byte(&[7, 7, 7, 7, 7, 7, 7, 7, 7]), Some(7));
    assert_eq!(rle_byte(&[7, 7, 7, 7, 7, 7, 7, 8]), None);
    assert_eq!(rle_byte(&[1]), None);
    let mut v = vec![0xAAu8; 1024];
    assert_eq!(rle_byte(&v), Some(0xAA));
    v[1000] = 0xAB;
    assert_eq!(rle_byte(&v), None);
}

#[test]
fn empty_frame_has_checksum() {
    let zst = compress(b"", 3).unwrap();
    assert!(zst.len() >= 13);
    assert_eq!(decompress(&zst).unwrap(), b"");
}

#[test]
fn roundtrip_all_strategies() {
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(32);
    let noise = xorshift(0x3333_4444, 4096);
    for id in 1i32..=9 {
        let mut params = crate::compression_params(3, Some(fox.len() as u64)).unwrap();
        params.apply_zstd_kv("strategy", id).unwrap();
        let zeros = [0u8; 2048];
        for src in [fox.as_slice(), noise.as_slice(), zeros.as_slice()] {
            let zst = compress_with_params(src, params, true).expect("compress");
            let got = decompress(&zst)
                .unwrap_or_else(|e| panic!("strategy {id} src={}: {e:?}", src.len()));
            assert_eq!(got, src, "strategy {id}");
        }
    }
}

#[test]
fn roundtrip_ultra_levels() {
    let src = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(48);
    for level in [20, 21, 22] {
        rt(&src, level);
        rt(&xorshift(0xABCDu64, 2048), level);
    }
}

#[test]
fn rle_fse_large_match() {
    let chunk: Vec<u8> = (0..32_768).map(|i| (i % 251) as u8).collect();
    let mut src = chunk.clone();
    src.extend_from_slice(&chunk);
    let zst = compress(&src, 1).expect("compress");
    let got = decompress(&zst).unwrap_or_else(|e| panic!("zst={} err={e:?}", zst.len()));
    assert_eq!(got, src);
}

#[test]
fn literals_and_sequence_modes_coverage() {
    let mut seen_lit = [false; 4];
    let mut seen_seq = [false; 4];
    let mut seen_4stream = false;
    let mut seen_huff_direct = false;
    let mut seen_huff_fse = false;

    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text_block = Vec::new();
    while text_block.len() < 64 * 1024 {
        text_block.extend_from_slice(fox);
    }
    let mut text_two_blocks = text_block.clone();
    while text_two_blocks.len() < 130 * 1024 {
        text_two_blocks.extend_from_slice(fox);
    }

    let mut skip = crate::compression_params(1, Some(400)).unwrap();
    skip.target_length = 1 << 16;
    skip.min_match = 7;
    skip.strategy = crate::Strategy::Fast;

    let mut huff_src = Vec::new();
    while huff_src.len() < 400 {
        huff_src.extend_from_slice(fox);
    }
    huff_src.truncate(400);
    let mut huff_two = Vec::new();
    while huff_two.len() < 130 * 1024 {
        huff_two.extend_from_slice(fox);
    }
    // Bytes 0 and 1: Huffman alphabet is tiny, so the tree is direct 4-bit
    // (FSE weight compression needs more than two weights).
    let mut two_sym = Vec::new();
    while two_sym.len() < 400 {
        two_sym.extend_from_slice(&[0u8, 0, 0, 1]);
    }
    two_sym.truncate(400);

    let mut rle_lits = fox.repeat(20);
    rle_lits.truncate(1024);
    for _ in 0..30 {
        rle_lits.push(0xA5);
        rle_lits.push(0xA5);
        rle_lits.extend_from_slice(&fox[..20]);
    }
    let mut rle_win = crate::compression_params(1, Some(rle_lits.len() as u64)).unwrap();
    rle_win.window_log = 10;

    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut blocks_log: Vec<(u8, Option<u8>)> = Vec::new();
    let zst_rl = compress_with_params(&rle_lits, rle_win, true).expect("rle lits");
    assert_eq!(decompress(&zst_rl).unwrap(), rle_lits);
    frames.push(zst_rl);
    for (src, p) in [
        (huff_src.as_slice(), skip),
        (huff_two.as_slice(), skip),
        (two_sym.as_slice(), skip),
    ] {
        let zst = compress_with_params(src, p, true).expect("huff params");
        assert_eq!(decompress(&zst).unwrap(), src);
        frames.push(zst);
    }

    let mut mixed = Vec::new();
    let mut n = 0u32;
    while mixed.len() < 4096 {
        mixed.extend_from_slice(b"block ");
        mixed.push(b'0' + (n % 10) as u8);
        mixed.extend_from_slice(b" extra words for matches ");
        mixed.extend_from_slice(&(n.to_le_bytes()));
        mixed.push(b'\n');
        n += 1;
    }
    frames.push({
        let z = compress(&mixed, 1).expect("mixed L1");
        assert_eq!(decompress(&z).unwrap(), mixed);
        z
    });
    let mut mix_win = crate::compression_params(1, Some(8192)).unwrap();
    mix_win.window_log = 12;
    let mixed2 = mixed.repeat(2);
    frames.push({
        let z = compress_with_params(&mixed2, mix_win, true).expect("mixed2");
        assert_eq!(decompress(&z).unwrap(), mixed2);
        z
    });
    let mut fox_win = crate::compression_params(3, Some(3000)).unwrap();
    fox_win.window_log = 10;
    let fox_multi = fox.repeat(60);
    let zst_rep = compress_with_params(&fox_multi, fox_win, true).expect("repeat seq");
    assert_eq!(decompress(&zst_rep).unwrap(), fox_multi);
    frames.push(zst_rep);
    let mut small_win = crate::compression_params(1, Some(2048)).unwrap();
    small_win.window_log = 10;
    let repeated = b"TheQuickBrownFox0123456789ABCD".repeat(80);
    let zst_rle = compress_with_params(&repeated, small_win, true).expect("rle seq");
    assert_eq!(decompress(&zst_rle).unwrap(), repeated);
    frames.push(zst_rle);

    // RLE-literals coverage: EVERY literal in the block must be the same
    // byte while sequences still exist. The patterns are primed via a
    // PREFIX so the block itself contains only matches plus single 'q'
    // separators; a 1-byte separator cannot be repcode-matched (that needs
    // 4 bytes), so brick 40 leaves it as a literal. The older corpus
    // reached this state via a matcher weakness that repcode-1 removed.
    let mut rle_prefix = Vec::new();
    let mut rle_body = Vec::new();
    for i in 0..24u8 {
        let pat: Vec<u8> = (0..24u8).map(|j| b'A' + ((i * 7 + j * 3) % 26)).collect();
        rle_prefix.extend_from_slice(&pat);
        rle_body.push(b'q');
        rle_body.extend_from_slice(&pat);
    }
    let zst_rle_lits = compress_using_prefix(&rle_body, &rle_prefix, 1).expect("rle lits prefix");
    assert_eq!(
        crate::decode::decompress_using_prefix(&zst_rle_lits, &rle_prefix).unwrap(),
        rle_body
    );
    frames.push(zst_rle_lits);

    // Repeat FSE mode (seq mode 3) needs CONSECUTIVE blocks whose sequence
    // statistics are close enough that reusing the previous table beats
    // rebuilding. That needs >1 block (128 KiB each) of STATIONARY content.
    // The small samples below cannot reach it, and repcode-1 search
    // (brick 40) shifted the distributions that used to hit it by luck.
    let mut stationary = Vec::with_capacity(400 * 1024);
    {
        let mut st = 0x2545_F491_4F6C_DD1Du64;
        while stationary.len() < 400 * 1024 {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            // 16-symbol alphabet: compressible, but no long matches, so
            // every block sees the same LL/ML/OF shape.
            for k in 0..8 {
                stationary.push(b'a' + ((st >> (k * 8)) & 0x0F) as u8);
            }
        }
    }
    let zst_stat = compress(&stationary, 1).expect("stationary");
    assert_eq!(decompress(&zst_stat).unwrap(), stationary);
    frames.push(zst_stat);

    // FSE Repeat mode (seq mode 3) needs a distribution that is STATIONARY
    // across blocks but NOT degenerate. `text_two_blocks` used to supply it,
    // but once repcode-1 search shipped (brick 67) that content became a
    // single constant-offset repeat, so its blocks select RLE (one symbol)
    // instead of Repeat -- a coverage fixture hostage to matcher quality,
    // the same trap the RLE-literals case hit earlier.
    //
    // Built here by construction instead: fox fragments of VARYING length
    // give varied litlen/matchlen codes, while the length distribution stays
    // identical block to block, so block N+1's cheapest table is block N's.
    let mut stationary = Vec::new();
    let mut rng = 0x1234_5678_9abc_def0u64;
    while stationary.len() < 400 * 1024 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let take = 12 + (rng >> 40) as usize % (fox.len() - 12);
        stationary.extend_from_slice(&fox[..take]);
    }

    let samples: Vec<(Vec<u8>, i32)> = vec![
        (fox[..20].to_vec(), 1),
        (b"TheQuickBrownFox0123456789ABCD".repeat(2), 1),
        (fox.repeat(4), 1),
        (fox.repeat(8), 3),
        (text_block.clone(), 3),
        (text_two_blocks, 3),
        (stationary, 3),
        (xorshift(0xF00Du64, 4096), 1),
        (vec![0u8; 8192], 1),
        (vec![b'a'; 1024], 5),
        (xorshift(0xF00Du64, 8192), 9),
    ];
    for (src, level) in samples {
        let zst = compress(&src, level)
            .unwrap_or_else(|e| panic!("compress L{level} src={}: {e:?}", src.len()));
        let got = decompress(&zst).unwrap_or_else(|e| {
            panic!(
                "decompress L{level} src={} zst={}: {e:?}",
                src.len(),
                zst.len()
            )
        });
        assert_eq!(got, src, "L{level} src={}", src.len());
        frames.push(zst);
    }
    for zst in &frames {
        for b in inspect_compressed_blocks(zst) {
            seen_lit[b.lit as usize] = true;
            if let Some(m) = b.seq {
                seen_seq[((m >> 6) & 3) as usize] = true;
                seen_seq[((m >> 4) & 3) as usize] = true;
                seen_seq[((m >> 2) & 3) as usize] = true;
            }
            if b.four_stream {
                seen_4stream = true;
            }
            match b.huff_tree {
                Some(true) => seen_huff_fse = true,
                Some(false) => seen_huff_direct = true,
                None => {}
            }
            blocks_log.push((b.lit, b.seq));
        }
    }

    // RLE literals (type 1) are gated directly by
    // `huffman::tests::rle_literals_section_emits_type_1_and_round_trips`.
    // Requiring them to FALL OUT of the match finder here made this test a
    // hostage to matcher quality: repcode-1 search (brick 40) legitimately
    // consumes the single-byte runs that used to survive as literals, so
    // the mode became unreachable from this corpus while the emit path
    // itself is unchanged.
    assert!(
        seen_lit[0],
        "missing Raw literals (type 0); lit={seen_lit:?} seq={seen_seq:?}"
    );
    assert!(
        seen_lit[2],
        "missing Huffman Compressed literals (type 2); lit={seen_lit:?} seq={seen_seq:?}"
    );
    assert!(
        seen_lit[3],
        "missing Treeless Huffman literals (type 3); lit={seen_lit:?} seq={seen_seq:?}"
    );
    assert!(
        seen_seq[0],
        "missing Predefined FSE mode; lit={seen_lit:?} seq={seen_seq:?}"
    );
    assert!(
        seen_seq[1],
        "missing RLE FSE mode; lit={seen_lit:?} seq={seen_seq:?} blocks={blocks_log:?}"
    );
    assert!(
        seen_seq[2],
        "missing Compressed FSE mode; lit={seen_lit:?} seq={seen_seq:?} blocks={blocks_log:?}"
    );
    assert!(
        seen_seq[3],
        "missing Repeat FSE mode; lit={seen_lit:?} seq={seen_seq:?}"
    );
    assert!(
        seen_4stream,
        "missing 4-stream Huffman; lit={seen_lit:?} seq={seen_seq:?}"
    );
    assert!(
        seen_huff_direct,
        "missing direct Huffman tree (header>=128); lit={seen_lit:?}"
    );
    assert!(
        seen_huff_fse,
        "missing FSE Huffman tree (header<128); lit={seen_lit:?}"
    );
}

struct InspectedBlock {
    lit: u8,
    seq: Option<u8>,
    four_stream: bool,
    /// `Some(true)` = FSE-compressed weights; `Some(false)` = direct 4-bit.
    huff_tree: Option<bool>,
}

fn inspect_compressed_blocks(zst: &[u8]) -> Vec<InspectedBlock> {
    use crate::block::{parse_block_header, BlockType};
    use crate::frame::parse_kind;
    use crate::reader::Reader;
    let mut r = Reader::new(zst);
    parse_kind(&mut r).expect("frame header");
    let mut out = Vec::new();
    loop {
        let bh = parse_block_header(&mut r).expect("block header");
        let payload = r.take(bh.payload_len() as usize).expect("payload");
        if bh.ty == BlockType::Compressed {
            let lit = payload[0] & 3;
            let size_fmt = (payload[0] >> 2) & 3;
            let four_stream = matches!(lit, 2 | 3) && size_fmt != 0;
            let huff_tree = if lit == 2 {
                let hlen = match size_fmt {
                    0 | 1 => 3usize,
                    2 => 4,
                    3 => 5,
                    _ => 0,
                };
                payload.get(hlen).map(|&b| b < 128)
            } else {
                None
            };
            let after = skip_literals_section(payload).expect("literals");
            let (nseq, rest) = read_nseq(after);
            let mode = if nseq == 0 {
                None
            } else {
                rest.first().copied()
            };
            out.push(InspectedBlock {
                lit,
                seq: mode,
                four_stream,
                huff_tree,
            });
        }
        if bh.last {
            break;
        }
    }
    out
}

fn skip_literals_section(payload: &[u8]) -> Option<&[u8]> {
    let first = *payload.first()?;
    let lit_type = first & 3;
    let size_fmt = (first >> 2) & 3;
    match lit_type {
        0 | 1 => {
            let (regen, hdr) = match size_fmt {
                0 | 2 => (u32::from(first >> 3), 1usize),
                1 => {
                    let b1 = *payload.get(1)?;
                    (u32::from(first >> 4) + (u32::from(b1) << 4), 2)
                }
                3 => {
                    let b1 = *payload.get(1)?;
                    let b2 = *payload.get(2)?;
                    (
                        u32::from(first >> 4) + (u32::from(b1) << 4) + (u32::from(b2) << 12),
                        3,
                    )
                }
                _ => return None,
            };
            let body = if lit_type == 1 {
                1usize
            } else {
                regen as usize
            };
            payload.get(hdr + body..)
        }
        2 | 3 => {
            let (csize, hdr) = match size_fmt {
                0 | 1 => {
                    let b1 = *payload.get(1)?;
                    let b2 = *payload.get(2)?;
                    let csize = ((u32::from(b1) >> 6) + (u32::from(b2) << 2)) & 0x3FF;
                    (csize as usize, 3usize)
                }
                2 => {
                    let b2 = *payload.get(2)?;
                    let b3 = *payload.get(3)?;
                    let csize = (u32::from(b2) >> 2) + (u32::from(b3) << 6);
                    ((csize as usize) & 0x3FFF, 4)
                }
                3 => {
                    let b2 = *payload.get(2)?;
                    let b3 = *payload.get(3)?;
                    let b4 = *payload.get(4)?;
                    let csize = (u32::from(b2) >> 6) + (u32::from(b3) << 2) + (u32::from(b4) << 10);
                    ((csize as usize) & 0x3FFFF, 5)
                }
                _ => return None,
            };
            payload.get(hdr + csize..)
        }
        _ => None,
    }
}

fn read_nseq(src: &[u8]) -> (u32, &[u8]) {
    let Some(&b0) = src.first() else {
        return (0, src);
    };
    if b0 == 0 {
        (0, &src[1..])
    } else if b0 < 128 {
        (u32::from(b0), &src[1..])
    } else if b0 < 255 {
        let b1 = src.get(1).copied().unwrap_or(0);
        (
            ((u32::from(b0) - 128) << 8) + u32::from(b1),
            src.get(2..).unwrap_or(&[]),
        )
    } else {
        let b1 = src.get(1).copied().unwrap_or(0);
        let b2 = src.get(2).copied().unwrap_or(0);
        (
            0x7F00 + u32::from(b1) + (u32::from(b2) << 8),
            src.get(3..).unwrap_or(&[]),
        )
    }
}

fn xorshift(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed;
    let mut v = vec![0u8; n];
    for b in &mut v {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s & 0xFF) as u8;
        if *b == 0 {
            *b = 1;
        }
    }
    v
}

fn count_zstd_blocks(zst: &[u8]) -> usize {
    let mut r = crate::reader::Reader::new(zst);
    crate::frame::parse_kind(&mut r).expect("frame header");
    let mut n = 0usize;
    loop {
        let h = crate::block::parse_block_header(&mut r).expect("block header");
        n += 1;
        r.take(h.payload_len() as usize).expect("payload");
        if h.last {
            break;
        }
    }
    n
}

#[test]
fn long_forces_window_descriptor() {
    let src = xorshift(0x5E1A_B1E5, 300 * 1024);
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
            ldm: crate::ldm::LdmParams::enabled(),
            ..AdvancedOptions::default()
        },
    )
    .expect("long compress");
    match crate::get_frame_header(&zst).expect("hdr") {
        crate::FrameKind::Zstd(h) => {
            assert!(
                !h.single_segment,
                "300 KiB > 256 KiB window must emit Window_Descriptor"
            );
            assert_eq!(h.window_size, 1u64 << 18);
        }
        other => panic!("expected zstd frame, got {other:?}"),
    }
    assert_eq!(decompress(&zst).expect("decode"), src);
}

#[test]
fn enable_ldm_zstd_keys_roundtrip() {
    let src = xorshift(0x1D1D, 32 * 1024);
    let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
    params
        .apply_zstd_option_string("enableLdm=1,ldmHashLog=12,ldmMinMatch=64,ldmHashRateLog=7")
        .unwrap();
    let ldm = params.ldm_params();
    assert!(ldm.enable);
    assert_eq!(ldm.hash_log, 12);
    assert_eq!(ldm.min_match, 64);
    let zst = compress_with_advanced(
        &src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions {
            ldm,
            ..AdvancedOptions::default()
        },
    )
    .expect("enableLdm compress");
    assert_eq!(decompress(&zst).expect("decode"), src);
}

#[test]
fn rsyncable_splits_blocks() {
    let src = xorshift(0xA11, 64 * 1024);
    let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
    params.window_log = 18;
    let plain = compress_with_advanced(
        &src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions::default(),
    )
    .unwrap();
    let rsync = compress_with_advanced(
        &src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions {
            ldm: crate::ldm::LdmParams::enabled(),
            rsyncable: true,
            target_cblock_size: 0,
            ..AdvancedOptions::default()
        },
    )
    .unwrap();
    let n_plain = count_zstd_blocks(&plain);
    let n_rsync = count_zstd_blocks(&rsync);
    assert_eq!(decompress(&rsync).unwrap(), src);
    assert!(
        n_rsync > n_plain,
        "rsyncable should cut extra blocks (plain={n_plain} rsync={n_rsync})"
    );
}

#[test]
fn target_cblock_caps_uncompressed_blocks() {
    let src = xorshift(0xC0B1, 32 * 1024);
    let params = compression_params(1, Some(src.len() as u64)).unwrap();
    let plain = compress_with_params(&src, params, true).unwrap();
    let capped = compress_with_advanced(
        &src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions {
            target_cblock_size: 256,
            ..AdvancedOptions::default()
        },
    )
    .unwrap();
    let n_plain = count_zstd_blocks(&plain);
    let n_capped = count_zstd_blocks(&capped);
    assert_eq!(decompress(&capped).unwrap(), src);
    assert!(
        n_capped > n_plain,
        "target cblock 256 => ~1 KiB raw blocks (plain={n_plain} capped={n_capped})"
    );
}

#[test]
fn decompress_long_raises_window_cap() {
    let src = xorshift(0x5716, 128 * 1024 + 64);
    let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
    params.window_log = 16;
    let zst = compress_with_params(&src, params, true).unwrap();
    match crate::get_frame_header(&zst).unwrap() {
        crate::FrameKind::Zstd(h) => {
            assert!(!h.single_segment);
            assert_eq!(h.window_size, 1u64 << 16);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        crate::decompress_with(
            &zst,
            crate::DecompressOptions {
                window_max: 32 * 1024,
                ..Default::default()
            }
        )
        .unwrap_err(),
        crate::Error::WindowTooLarge
    );
    assert_eq!(
        crate::decompress_with(
            &zst,
            crate::DecompressOptions {
                window_max: 1u64 << 16,
                ..Default::default()
            }
        )
        .unwrap(),
        src
    );
}

#[test]
fn fast_sparse_match_fill_roundtrips_repeating_text() {
    let src = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(8000);
    for level in [1, -1, -4] {
        rt(&src, level);
        let zst = compress(&src, level).unwrap();
        assert!(
            zst.len() < src.len() / 20,
            "L{level}: repeating text should stay compact ({} vs {})",
            zst.len(),
            src.len()
        );
    }
}

#[test]
fn count_match_words_match_byte_loop() {
    fn bytes(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
        let max = (limit - ip).min(src.len() - m).min(src.len() - ip);
        let mut n = 0usize;
        while n < max && src[m + n] == src[ip + n] {
            n += 1;
        }
        n
    }
    let mut src = vec![0u8; 4096];
    for (i, b) in src.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let head: Vec<u8> = src[0..200].to_vec();
    src[200..400].copy_from_slice(&head);
    let mid: Vec<u8> = src[3..20].to_vec();
    src[800..800 + 17].copy_from_slice(&mid);
    for m in [0usize, 1, 3, 7, 8, 15, 200] {
        for ip in [200usize, 201, 400, 800, 801, 2000] {
            if m >= src.len() || ip >= src.len() {
                continue;
            }
            for limit in [ip, ip + 1, ip + 7, ip + 8, ip + 9, ip + 64, src.len()] {
                let limit = limit.min(src.len());
                if ip > limit {
                    continue;
                }
                assert_eq!(
                    count_match(&src, m, ip, limit),
                    bytes(&src, m, ip, limit),
                    "m={m} ip={ip} limit={limit}"
                );
            }
        }
    }
}

#[test]
fn min_gain_matches_c_fast() {
    assert_eq!(
        min_gain(128 * 1024, Strategy::Fast),
        ((128 * 1024) >> 6) + 2
    );
    assert_eq!(min_gain(100, Strategy::Greedy), (100 >> 6) + 2);
    // Written as the formula, not its value: the point of the assert is that
    // `min_gain` IS `(src >> shift) + 2` at BtUltra's shift of 7.
    #[allow(clippy::identity_op)]
    let bt_ultra_100 = (100 >> 7) + 2;
    assert_eq!(min_gain(100, Strategy::BtUltra), bt_ultra_100);
}

#[test]
fn early_raw_skip_fast_rung_low_matches() {
    SKIP_OVERRIDE.with(|c| c.set(None));
    let fast = compression_params(-1, Some(128 * 1024)).unwrap();
    assert!(fast.target_length >= 1);
    assert!(fast.target_length <= 7);
    let mg = min_gain(128 * 1024, fast.strategy);
    assert!(early_raw_skip(mg.saturating_sub(1), 128 * 1024, fast));
    assert!(!early_raw_skip(mg + 10, 128 * 1024, fast));
    let l1 = compression_params(1, Some(128 * 1024)).unwrap();
    assert_eq!(l1.target_length, 0);
    assert!(!early_raw_skip(0, 128 * 1024, l1));
    let l3 = compression_params(3, Some(128 * 1024)).unwrap();
    assert!(l3.strategy != Strategy::Fast);
    assert!(!early_raw_skip(0, 128 * 1024, l3));
}

#[test]
fn skip_off_l1_bytes_match_unset() {
    let src = xorshift(0xBEEF, 32 * 1024);
    SKIP_OVERRIDE.with(|c| c.set(Some(false)));
    let off = compress(&src, 1).expect("off");
    SKIP_OVERRIDE.with(|c| c.set(None));
    let unset = compress(&src, 1).expect("unset");
    assert_eq!(off, unset, "knob-off at -1 must match default (tlen=0)");
    assert_eq!(decompress(&off).unwrap(), src);
}

#[test]
fn skip_off_fast_roundtrip_and_on_skips_noise() {
    let src = xorshift(0xA11E, 64 * 1024);
    SKIP_OVERRIDE.with(|c| c.set(Some(false)));
    let off = compress(&src, -1).expect("off");
    SKIP_OVERRIDE.with(|c| c.set(Some(true)));
    let on = compress(&src, -1).expect("on");
    SKIP_OVERRIDE.with(|c| c.set(None));
    assert_eq!(decompress(&off).unwrap(), src);
    assert_eq!(decompress(&on).unwrap(), src);
    let l1 = compress(&src, 1).expect("l1");
    assert_eq!(decompress(&l1).unwrap(), src);
    let off_c = frame_block_census(&off).unwrap();
    let on_c = frame_block_census(&on).unwrap();
    assert!(
        on_c.raw >= off_c.raw,
        "skip-on should dump at least as many raw blocks"
    );
}
