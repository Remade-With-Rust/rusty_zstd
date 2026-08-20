//! EVERY variable in the pair_route gate, per corpus, at L1.
//!
//! Decision order (encode.rs:3037):
//!   route_force != 0            -> route_force      (test override, 0 in prod)
//!   step_pick==2 && reprobe>0   -> 2                (4.70 probe, default OFF)
//!   !pair_enabled()             -> 0                (RZSTD_PAIR, default on)
//!   target_length != 0          -> 2                (0 at L1)
//!   pair_probe == 0             -> 2                (forced probe every 16 blocks)
//!   pair_gain <  pair_gain_min()-> 0                (default 0.20)
//!   pair_gain >= pair_rate_hi() -> 2                (default 1.00)
//!   rep_yield >  pair_rep_max() -> 0                (default 0.70)
//!   else                        -> 1
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("GATE 18 @ L1 -- every gate variable, per corpus\n");
    println!("thresholds: pair_gain_min 0.20   pair_rate_hi 1.00   pair_rep_max 0.70");
    println!("            PAIR_PROBE_PERIOD 16   step0_default 2\n");
    println!("{:<14}{:>7}{:>7}{:>7}{:>11}{:>11}{:>9}","corpus","rt 0","rt 1","rt 2","pair_gain","rep_yield","pair%");
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let _=rusty_zstd::take_route_hist(); let _=rusty_zstd::take_pair_stats();
        let _=rusty_zstd::compress(src,1).unwrap();
        let (r0,r1,r2,g,y)=rusty_zstd::take_route_hist();
        let t=(r0+r1+r2).max(1);
        println!("{id:<14}{r0:>7}{r1:>7}{r2:>7}{g:>11.4}{y:>11.4}{:>8.1}%",
            100.0*r2 as f64/t as f64);
    }
    println!("\nrt0 = no pair, step 2   rt1 = no pair, step 1   rt2 = PAIR + step 2");
    println!("(the pair search additionally needs rep_yield <= 0.70 at encode.rs:3374)");
}
