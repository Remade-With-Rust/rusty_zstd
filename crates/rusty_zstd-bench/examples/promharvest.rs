//! PROMETHEUS STAGE 1 -- harvest the literals gate's confusion matrix.
//!
//! `literals_worth_huffman` is a fitted heuristic with two hand-derived
//! constants (`distinct*2 >= len` -> reject; `sum_sq*128 >= n^2` -> accept,
//! i.e. collision entropy H2 <= 7 bits/symbol). It gates an O(n) histogram + a
//! ctable build + `write_tree` on EVERY block, and it has no term for the
//! DECODE cost it spends (m7-anatomy S4.4).
//!
//! SEMANTICS, READ OFF THE CALL SITES (huffman.rs:1887 etc), not guessed:
//!   * `note_lit_try(0)` fires AFTER the gate's early return, so LIT_TRY[0] is
//!     the count of blocks the gate ACCEPTED -- not the total.
//!   * LIT_TRY[6] ("SKIPPED") is the `futile` PREV-TABLE skip, a DIFFERENT gate.
//!   * LIT_TRY[5] ("raw_won") = accepted, built a ctable, and raw still won:
//!     the wasted-work column.
//! Total blocks comes from EncodeCounts (comp+raw+rle), not from LIT_TRY.
//!
//! FALSE-ACCEPT RATE = raw_won / new_ENCODED is the wasted-work column.
//! It decides whether a better gate is worth discovering at all.
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray","versions-16m",
    "text-32m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn field(d:&str,k:&str)->u64{
    for line in d.lines(){
        if let Some(r)=line.strip_prefix("lit_try "){
            for kv in r.split_whitespace(){
                if let Some((a,b))=kv.split_once('='){
                    if a==k { return b.parse().unwrap_or(0) }
                }
            }
        }
    }
    0
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    let cap=8usize<<20;
    println!("PROMETHEUS harvest -- literals gate confusion @ L{lvl}\n");
    println!("| corpus | total blocks | gate ACCEPTED | gate REJECTED | reject % | huff won | raw won (wasted) | **false-accept %** |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    let (mut tb,mut ts,mut tn,mut tw,mut tr,mut tp)=(0u64,0u64,0u64,0u64,0u64,0u64);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let s=&f[..f.len().min(cap)];
        rusty_zstd::prof_reset();
        let _=rusty_zstd::compress(s,lvl).unwrap();
        let d=rusty_zstd::prof_dump();
        let c=rusty_zstd::prof_encode_counts();
        let total=c.comp_blocks+c.raw_blocks+c.rle_blocks;
        let acc=field(&d,"blocks");                 // gate-ACCEPTED only
        let rej=total.saturating_sub(acc);
        let rw=field(&d,"raw_won");
        let won=acc.saturating_sub(rw);
        if total==0 {continue}
        tb+=total; ts+=rej; tn+=acc; tw+=won; tr+=rw; tp+=0;
        println!("| {id} | {total} | {acc} | {rej} | {:.1} | {won} | {rw} | **{:.1}** |",
            100.0*rej as f64/total as f64,
            if acc>0 {100.0*rw as f64/acc as f64} else {0.0});
    }
    println!("| **board** | **{tb}** | **{ts}** | **{tn}** | **{tw}** | **{tr}** | **{:.1}** | **{tp}** |",
        if tn>0 {100.0*tr as f64/tn as f64} else {0.0});
    println!("\nskip rate {:.1}%  |  of the blocks that PAID for a ctable build, {:.1}% were wasted",
        if tb>0 {100.0*ts as f64/tb as f64} else {0.0},
        if tn>0 {100.0*tr as f64/tn as f64} else {0.0});
}
