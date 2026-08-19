//! Did GATE 14's work count price only one side? MM_TOTAL counts main-loop
//! POSITIONS. Raising the cut makes the next-long PROBE fire more often, and
//! each firing is a hash lookup + match_ok + count_match that MM_TOTAL never
//! sees. Count both.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("{:<14}{:>12}{:>12}{:>11}{:>13}{:>13}{:>10}","corpus","pos before","pos after","pos %","nl before","nl after","nl %");
    let (mut pa,mut pb,mut na,mut nb)=(0u64,0u64,0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(8); rusty_zstd::set_dfast_good_ml2_arm(8);
        rusty_zstd::set_nl_off_worse_arm(-1.0);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_next_long();
        let _=rusty_zstd::compress(src,3).unwrap();
        let x=rusty_zstd::take_mm().0; let (n1,_,_)=rusty_zstd::take_next_long();
        rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
        rusty_zstd::set_nl_off_worse_arm(0.60);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_next_long();
        let _=rusty_zstd::compress(src,3).unwrap();
        let y=rusty_zstd::take_mm().0; let (n2,_,_)=rusty_zstd::take_next_long();
        pa+=x; pb+=y; na+=n1; nb+=n2;
        println!("{id:<14}{x:>12}{y:>12}{:>10.2}%{n1:>13}{n2:>13}{:>9.1}%",
            100.0*(y as f64-x as f64)/x as f64, 100.0*(n2 as f64-n1 as f64)/n1.max(1) as f64);
    }
    println!("\nTOTAL positions {pa} -> {pb} ({:+.2}%)   next-long probes {na} -> {nb} ({:+.1}%)",
        100.0*(pb as f64-pa as f64)/pa as f64, 100.0*(nb as f64-na as f64)/na as f64);
    println!("net op change: {:+}", (pb as i64 - pa as i64) + (nb as i64 - na as i64));
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
    rusty_zstd::set_nl_off_worse_arm(0.60);
}
