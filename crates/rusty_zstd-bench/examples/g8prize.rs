//! Gate 8 covers only 48% of find_fast calls -- Gate 6's pair route runs the
//! NON-pipelined loop. How much speculation would that loop consume?
const IDS: &[&str] = &["mr","dickens","sao","ooffice","mozilla","samba","x-ray","osdb","webster","reymont","nci","xml","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","text-32m","zeros-32m"];
fn main(){
    println!("{:<14}{:>14}{:>14}{:>9}", "corpus","main pos","reach advance","use%");
    println!("{}","-".repeat(52));
    let (mut tt,mut tm)=(0u64,0u64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let _=rusty_zstd::take_mm();
        let _=rusty_zstd::compress(src,1).unwrap();
        let (tot,miss)=rusty_zstd::take_mm();
        if tot>0 { println!("{id:<14}{tot:>14}{miss:>14}{:>8.1}%", 100.0*miss as f64/tot as f64); }
        tt+=tot; tm+=miss;
    }
    println!("\nmain-loop positions {tt}, reaching the advance {tm} ({:.1}%)",
        100.0*tm as f64/tt.max(1) as f64);
    println!("-> a speculation in this loop would be CONSUMED {:.1}% of the time",
        100.0*tm as f64/tt.max(1) as f64);
}
