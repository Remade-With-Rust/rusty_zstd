//! The GREEDY parse of a ROW frame: libzstd's `ZSTD_compressBlock_greedy_row`
//! (`ZSTD_compressBlock_lazy_generic` at depth 0 over `ZSTD_RowFindBestMatch`)
//! as ONE function.
//!
//! `find_lazy_rows` at depth 0 makes the same decisions, and this finder is
//! held to them: same candidates, same order, same accept rule, same repcode
//! bookkeeping. What differs is the SHAPE, and it is libzstd's:
//!
//! * the search is inlined here -- `row_find_best` is an outlined fn-pointer
//!   kernel that re-reads a 24-field `ChainCtx` on every call;
//! * every inserted or searched position names the row of the position
//!   `HC` on (libzstd's `hashCache` distance). `find_lazy_rows` named one
//!   row ahead of each search and nothing ahead of the back-fill;
//! * the back-fill is the loop below rather than an outlined call per match.
use super::*;

/// How far ahead of its use a row is named -- libzstd's
/// `ZSTD_ROW_HASH_CACHE_SIZE`.
const HC: usize = 8;

/// The wide row key: the `mls`-byte gram, one multiply. Identical to
/// `hash_wide_link_tag_b(..).0`, which every other row producer uses.
#[inline(always)]
fn row_key(src: &[u8], p: usize, shift64: u32, smask: u64) -> usize {
    ((load_u64le(src, p) & smask).wrapping_mul(FAST_HASH_PRIME64) >> shift64) as usize
}

/// See the module comment. Output is identical to `find_lazy_rows::<RL>`
/// at depth 0 on the same table.
#[inline(never)]
#[allow(unsafe_code)]
pub(crate) fn find_greedy_rows<const RL: u32>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    debug_assert_eq!(RL, tables.rows.row_log());
    debug_assert!(tables.chain_wide);
    let mls = params.min_match.clamp(3, 7) as usize;
    let attempts = search_attempts(params);
    let (mut seqs, mut lits) = match chain_finder_prologue(src, block_start, block_end, tables, mls)
    {
        Ok(t) => t,
        Err(out) => return out,
    };
    // The prologue returned unless `block_start < block_end - 8`.
    let ilimit = block_end - 8;
    debug_assert!(block_end <= src.len());
    let prefix_lowest = block_start.saturating_sub(window).max(tables.frame_start);
    let lowest1 = prefix_lowest.max(1);
    let smask = (1u64 << (8 * mls)) - 1;
    let shift64 = 64u32.saturating_sub(tables.hash_log.min(32));
    let lp_copy = lit_width_for(tables);
    let accel_sh = lazy_step_shift(lazy_accel());
    let rep_skip = row_rep_skip();
    // C: `ip += (dictAndPrefixLength == 0)`.
    let mut ip = block_start + usize::from(block_start == prefix_lowest);
    let mut anchor = block_start;
    let mut offset_1 = reps[0] as usize;
    let mut offset_2 = reps[1] as usize;
    {
        let max_rep = ip - prefix_lowest;
        if offset_1 > max_rep {
            offset_1 = 0;
        }
        if offset_2 > max_rep {
            offset_2 = 0;
        }
    }
    let mut ntu = tables.row_ntu;
    if ntu > block_start {
        ntu = block_start;
    }
    if ntu < prefix_lowest {
        ntu = prefix_lowest;
    }
    let mut lazy_skipping = false;
    let mut searches = 0u64;
    let mut rep_hits = 0u64;

    // The row table, by base pointer. SAFETY (every access below): a row
    // index is `(h >> RL) & rmask` with `rmask == head.len() - 1`, so it is
    // `< head.len()`; `pos` and `tags` are `head.len() << RL` long and a slot
    // is `(r << RL) + s` with `s < 1 << RL`. Nothing in this function resizes
    // the table, and `tables` is not otherwise touched until the epilogue.
    let rmask = tables.rows.mask();
    debug_assert_eq!(rmask + 1, tables.rows.head.len());
    debug_assert_eq!(tables.rows.pos.len(), tables.rows.head.len() << RL);
    debug_assert_eq!(tables.rows.tags.len(), tables.rows.head.len() << RL);
    let posp = tables.rows.pos.as_mut_ptr();
    let tagp = tables.rows.tags.as_mut_ptr();
    let headp = tables.rows.head.as_mut_ptr();
    const N_MASK: usize = (1usize << crate::rowfind::ROW_LOG_MAX) - 1;
    let slot_mask: u32 = (1u32 << RL) - 1;

    // Name the lines a probe or an insert of row `r` touches. The head
    // array is one byte per row (32 KiB at L5's hash log), cache-resident,
    // and not named.
    macro_rules! prefetch_row {
        ($h:expr) => {{
            let r = ($h >> RL) & rmask;
            let at = r << RL;
            // SAFETY: `at + (1 << RL) <= pos.len() == tags.len()`; every
            // pointer formed is inside row `r`.
            unsafe {
                crate::simd::prefetch_raw(tagp.add(at) as *const u8);
                let pb = posp.add(at) as *const u8;
                let mut off = 0usize;
                while off < (4usize << RL) {
                    crate::simd::prefetch_raw(pb.add(off));
                    off += 64;
                }
                crate::simd::prefetch_raw(pb.add((4usize << RL) - 1));
            }
        }};
    }
    // Name the row of `q + HC` if that position will be hashed at all.
    // Positions past `ilimit` are never inserted or searched (which is also
    // what keeps every hashed `q + 8 <= src.len()`).
    macro_rules! name_ahead {
        ($q:expr) => {{
            let qa: usize = $q + HC;
            if qa <= ilimit {
                prefetch_row!(row_key(src, qa, shift64, smask));
            }
        }};
    }
    // Insert `q` (hash `h`) at its row's head.
    macro_rules! insert {
        ($q:expr, $h:expr) => {{
            let q: usize = $q;
            let r = ($h >> RL) & rmask;
            unsafe {
                let s = u32::from(*headp.add(r));
                debug_assert!(s <= slot_mask);
                let at = (r << RL) + s as usize;
                *posp.add(at) = q as u32;
                *tagp.add(at) = link_tag(src, q, mls);
                *headp.add(r) = ((s + 1) & slot_mask) as u8;
            }
        }};
    }
    // Insert `from .. to` -- libzstd's update with its long-match skip
    // (`row_catch_up`'s rule). Every inserted position names the row
    // `HC` positions on, libzstd's hash-cache distance; the loop is split
    // so the bulk runs without the `ilimit` test.
    macro_rules! fill_run {
        ($from:expr, $to:expr) => {{
            let (mut q, to): (usize, usize) = ($from, $to);
            let pf_end = to.min((ilimit + 1).saturating_sub(HC));
            while q < pf_end {
                prefetch_row!(row_key(src, q + HC, shift64, smask));
                insert!(q, row_key(src, q, shift64, smask));
                q += 1;
            }
            while q < to {
                insert!(q, row_key(src, q, shift64, smask));
                q += 1;
            }
        }};
    }
    macro_rules! fill {
        ($from:expr, $to:expr) => {{
            let (from, to): (usize, usize) = ($from, $to);
            if to - from > ROW_SKIP_GAP {
                fill_run!(from, from + ROW_SKIP_HEAD);
                fill_run!(to - ROW_SKIP_TAIL, to);
            } else {
                fill_run!(from, to);
            }
        }};
    }
    // One search at `p`: catch the table up, probe, insert `p`. Returns
    // `(match position, length)`, length 0 for none.
    macro_rules! search {
        ($p:expr) => {{
            let p: usize = $p;
            debug_assert!(p <= ilimit);
            if !lazy_skipping {
                if ntu < p {
                    fill!(ntu, p);
                }
                name_ahead!(p);
            }
            let h = row_key(src, p, shift64, smask);
            ntu = p + 1;
            searches += 1;
            let gtag = link_tag(src, p, mls);
            let r = (h >> RL) & rmask;
            let at = r << RL;
            // SAFETY: see the table note above.
            let (mask, rhead) = unsafe {
                (
                    crate::rowfind::row_tag_mask_raw::<RL>(tagp.add(at), gtag),
                    u32::from(*headp.add(r)),
                )
            };
            let mut w = crate::rowfind::row_rot::<RL>(mask, rhead);
            #[cfg(feature = "profile")]
            {
                use core::sync::atomic::Ordering::Relaxed;
                ROW_LOADS.fetch_add(1, Relaxed);
                ROW_BUCKET[4].fetch_add(1, Relaxed);
            }
            let low = lowest1.max(p.saturating_sub(window));
            let mut best_m = 0usize;
            let mut best_ml = 0usize;
            if p > low {
                let span = p - low;
                let mut cand =
                    [core::mem::MaybeUninit::<u32>::uninit(); 1 << crate::rowfind::ROW_LOG_MAX];
                let mut n = 0usize;
                // The newest `attempts` slots whose tag matches, valid or not
                // -- what `row_find_best` keeps by clearing the oldest set
                // bits first.
                let mut left = attempts;
                while w != 0 && left != 0 {
                    left -= 1;
                    let b = 63 - w.leading_zeros();
                    w &= !(1u64 << b);
                    let s = ((b + rhead) & slot_mask) as usize;
                    // SAFETY: `s < 1 << RL`, a slot of row `r`.
                    let m = unsafe { *posp.add(at + s) };
                    if (m as usize).wrapping_sub(low) >= span {
                        continue;
                    }
                    // `low <= m < p`, so in bounds.
                    crate::simd::prefetch_raw(unsafe { src.as_ptr().add(m as usize) });
                    cand[n & N_MASK].write(m);
                    n += 1;
                }
                insert!(p, h);
                #[cfg(feature = "profile")]
                {
                    crate::prof::note_probes(n as u64);
                    ROW_EXAM.fetch_add(n as u64, core::sync::atomic::Ordering::Relaxed);
                    ROW_BUCKET[0].fetch_add(n as u64, core::sync::atomic::Ordering::Relaxed);
                }
                let mut bar = mls;
                let mut i = 0usize;
                while i < n {
                    // SAFETY: `i < n`, and the gather wrote exactly `cand[0..n]`.
                    let m = unsafe { cand[i & N_MASK].assume_init() } as usize;
                    i += 1;
                    if let Some(x) = mls_xor(src, m, p, mls, smask) {
                        #[cfg(feature = "profile")]
                        ROW_BUCKET[2].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        if best_ml == 0 || pre_eq(src, m, p, best_ml) {
                            let ml = fused_ml(x, src, m, p, block_end);
                            if ml >= bar {
                                best_ml = ml;
                                best_m = m;
                                bar = ml + 1;
                                if p + best_ml >= block_end {
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                insert!(p, h);
            }
            (best_m, best_ml)
        }};
    }

    'outer: while ip <= ilimit {
        let mut ml = 0usize;
        // 0 = the match in hand is repcode 1.
        let mut off = 0usize;
        let mut start = ip + 1;
        'pick: {
            // Repcode 1 at `ip + 1`, taken at once (depth 0).
            if offset_1 != 0 {
                if let Some(l) = rep1_len_w(src, ip + 1, ip + 1 - offset_1, block_end, ip < ilimit)
                {
                    ml = l;
                    break 'pick;
                }
            }
            let (m, ml2) = search!(ip);
            if ml2 > ml {
                ml = ml2;
                start = ip;
                off = ip - m;
            }
            if ml == 0 {
                // C: "jump faster over incompressible sections".
                let step = ((ip - anchor) >> accel_sh) + 1;
                ip += step;
                lazy_skipping = step > ROW_LAZY_SKIP_STEP;
                continue 'outer;
            }
        }
        debug_assert!(ml >= mls.min(4) && start + ml <= block_end);
        let emit_off = if off != 0 {
            // C's "catch up": extend the match backwards over the literals.
            while start > anchor && start - off > prefix_lowest && back_eq(src, start, start - off)
            {
                start -= 1;
                ml += 1;
            }
            // After literals, an offset equal to `offset_1` is written as
            // repcode 1 and moves nothing (see `find_lazy_rows`).
            if start == anchor || off != offset_1 {
                offset_2 = offset_1;
                offset_1 = off;
            }
            off
        } else {
            rep_hits += 1;
            offset_1
        };
        push_literals(&mut lits, src, anchor, start, lp_copy);
        seqs.push(Seq {
            litlen: (start - anchor) as u32,
            matchlen: ml as u32,
            offset: emit_off as u32,
        });
        ip = start + ml;
        anchor = ip;
        lazy_skipping = false;
        if rep_skip && off == 0 {
            ntu = ip;
        }
        // Repcode 2 straight after a match, no literals: it becomes repcode 1.
        while ip <= ilimit && offset_2 != 0 {
            let Some(l) = rep1_len_w(src, ip, ip - offset_2, block_end, true) else {
                break;
            };
            core::mem::swap(&mut offset_1, &mut offset_2);
            rep_hits += 1;
            seqs.push(Seq {
                litlen: 0,
                matchlen: l as u32,
                offset: offset_1 as u32,
            });
            ip += l;
            anchor = ip;
            if rep_skip {
                ntu = ip;
            }
        }
    }
    tables.row_ntu = ntu;
    greedy_finder_epilogue(
        tables,
        src,
        &seqs,
        &mut lits,
        anchor,
        block_start,
        block_end,
        rep_hits,
        false,
        (0, 0),
        attempts,
        searches,
        0,
        seqs.len() as u64,
    );
    (seqs, lits)
}
