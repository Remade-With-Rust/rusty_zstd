//! `params.hash_log` is user-settable with NO upper bound (`hlog` in the
//! advanced-parameter setter). `MatchTables` clamps it to 24 for the ALLOCATION,
//! but find_lazy / find_greedy / chain_find_best / bt_find_best still index with
//! the RAW value via `self.hash[h]`. Brick 52 fixed only find_fast and
//! find_dfast. Does the unfixed path go out of bounds?
fn main(){
    let src: Vec<u8> = (0..4_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    for hlog in [16u32, 22, 24, 25, 26, 28, 30] {
        for lvl in [1i32, 3, 9, 13, 19] {
            let mut p = rusty_zstd::compression_params(lvl, Some(src.len() as u64)).unwrap();
            p.hash_log = hlog;
            let r = std::panic::catch_unwind(|| {
                rusty_zstd::compress_with_params(&src, p, false)
            });
            match r {
                Ok(Ok(z)) => {
                    let ok = rusty_zstd::decompress(&z).map(|d| d==src).unwrap_or(false);
                    println!("hlog {hlog:<3} L{lvl:<3} ok  {:>10} bytes  round-trip {}", z.len(), if ok {"OK"} else {"FAIL"});
                }
                Ok(Err(e)) => println!("hlog {hlog:<3} L{lvl:<3} Err {e:?}"),
                Err(_)     => println!("hlog {hlog:<3} L{lvl:<3} *** PANIC ***"),
            }
        }
    }
}
