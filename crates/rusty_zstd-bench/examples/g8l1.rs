//! GATE 8 @ L1 -- protocol step 1 (is it dead?) and the deterministic ledger.
//! The pipelined loop runs only when `PIPE && !pair`, and Gate 6's route now
//! sets `pair` on many blocks, so reachability must be MEASURED, not assumed.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    println!("{:<14}{:>9}{:>10}{:>13}{:>10}", "corpus","blocks","pipelined","speculated","use%");
    println!("{}","-".repeat(56));
    let (mut tb,mut tp,mut tm,mut tu)=(0u64,0u64,0u64,0u64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let _=rusty_zstd::take_ff_pipe(); let _=rusty_zstd::take_finder_calls();
        let z=rusty_zstd::compress(src,1).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} round-trip");
        let (blocks,made,used)=rusty_zstd::take_ff_pipe();
        let (calls,_)=rusty_zstd::take_finder_calls();
        println!("{id:<14}{calls:>9}{blocks:>10}{made:>13}{:>9.1}%",
            100.0*used as f64/made.max(1) as f64);
        tb+=calls; tp+=blocks; tm+=made; tu+=used;
    }
    println!("\nfind_fast calls {tb}, of which PIPELINED {tp} ({:.0}%)", 100.0*tp as f64/tb.max(1) as f64);
    println!("speculated {tm}, used {tu} ({:.1}%), WASTED {} ({:.1}%)",
        tm-tu, 100.0*(tm-tu) as f64/tm.max(1) as f64, 100.0*tu as f64/tm.max(1) as f64);
    println!("\n{}", if tp==0 {"GATE 8 @ L1 is DEAD -- Gate 6's route left the pipelined loop with no caller"}
                     else {"GATE 8 @ L1 is ALIVE"});
}
