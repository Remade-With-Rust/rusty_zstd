//! push_literals tier multiversioning: the per-CALL ledger. The widening saves
//! 2 instr on a 32B tier hit and 4 on a 64B hit, but EVERY call pays the
//! dispatch branch plus a vzeroupper -- including calls that fall through to the
//! fallback and widen nothing.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","versions-16m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    for lvl in [1i32,3]{
        let (mut t2,mut t3,mut slow)=(0u64,0u64,0u64);
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            let _=rusty_zstd::take_lit_push(); let _=rusty_zstd::take_lit_tiers();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            let (_f1,s)=rusty_zstd::take_lit_push();
            let (a,b)=rusty_zstd::take_lit_tiers();
            t2+=a; t3+=b; slow+=s;
        }
        let calls=t2+t3+slow;
        let saved=t2*2+t3*4;
        // per call: cached-atomic load + test + branch (~2) + vzeroupper (1)
        let cost=calls*3;
        println!("L{lvl}: tier2 {t2}, tier3 {t3}, fallback {slow}  -> {calls} calls");
        println!("     saved {saved} instr, dispatch+vzeroupper cost {cost} instr, NET {:+}\n",
            saved as i64 - cost as i64);
    }
}
