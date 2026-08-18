//! The pipelined loop claims byte-identity with the main loop. It runs only when
//! `PIPE && !pair`, so force pair OFF and toggle PIPE: both loops then do the
//! SAME work and any size difference is a genuine divergence.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for (label, mode) in [("STEP 2, pair OFF (both loops same work)", 0),
                          ("STEP 1, pair OFF (Gate 6's step-1 route)", 1),
                          ("Gate 6 route active (default)", 2)] {
        println!("\n=== {label} ===");
        let mut moved=0;
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(8<<20)];
            let set=|p:bool|{
                match mode {
                    0 => { rusty_zstd::set_pair_on_arm(false); rusty_zstd::set_step0_arm(2); }
                    1 => { rusty_zstd::set_pair_on_arm(false); rusty_zstd::set_step0_arm(1); }
                    _ => { rusty_zstd::set_pair_on_arm(true);  rusty_zstd::set_step0_arm(2); }
                }
                rusty_zstd::set_pipe_arm(p); };
            set(true);  let a=rusty_zstd::compress(src,1).unwrap().len();
            set(false); let b=rusty_zstd::compress(src,1).unwrap().len();
            if a!=b {
                moved+=1;
                println!("  {id:<14} pipe-ON {a:>9}  pipe-OFF {b:>9}  {:+.3}%",
                    100.0*(b as f64-a as f64)/a as f64);
            }
        }
        println!("  -> {moved}/18 diverge");
    }
    rusty_zstd::set_pair_on_arm(true); rusty_zstd::set_pipe_arm(true);
}
