//! Is `bt_find_best_runtime` -- a SECOND hand-written copy of the tree walk --
//! reachable in production? The duplicated-body class already broke Gate 4's
//! byte-identity once this campaign (`find_dfast_runtime`).
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let (mut ts,mut tr)=(0u64,0u64);
    println!("{:>5}{:>16}{:>16}","level","specialised","runtime");
    for lvl in [13,16,17,18,19,20,21,22]{
        let (mut sp,mut rt)=(0u64,0u64);
        for id in IDS{
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
            let src=&full[..full.len().min(512<<10)];
            let _=rusty_zstd::take_bt_calls();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            let (a,b)=rusty_zstd::take_bt_calls();
            sp+=a; rt+=b;
        }
        println!("{lvl:>5}{sp:>16}{rt:>16}");
        ts+=sp; tr+=rt;
    }
    println!("\nTOTAL specialised {ts}, runtime {tr}");
    // and with a hash_log the dispatcher has no arm for
    println!("\n=== with an unusual hash_log (user-settable, unbounded) ===");
    let src:Vec<u8>=(0..600_000u32).map(|i|(i.wrapping_mul(2_654_435_761)>>13) as u8).collect();
    for hl in [16u32,18,20,21,25,28] {
        let _=rusty_zstd::take_bt_calls();
        let mut p=rusty_zstd::compression_params(19,Some(src.len() as u64)).unwrap();
        p.hash_log=hl;
        let _=rusty_zstd::compress_with_params(&src,p,false).unwrap();
        let (a,b)=rusty_zstd::take_bt_calls();
        let m=if b>0 {"  <- RUNTIME BODY REACHED"} else {""};
        println!("hash_log {hl:>3}: specialised {a:>10}  runtime {b:>10}{m}");
    }
}
