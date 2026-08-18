//! Is the residual pipe-on/off divergence a LOOP difference or ROUTE feedback?
//! Pin the route so the adaptive term cannot vary; if divergence vanishes, the
//! loops are equivalent and the route sequence was what differed.
const IDS: &[&str] = &["mr","sao","mozilla","samba","x-ray","nci","xml","dickens"];
fn run(label:&str, pin: Option<f32>) {
    let mut moved=0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let set=|p:bool|{
            rusty_zstd::set_pair_on_arm(true);
            match pin {
                Some(g) => { rusty_zstd::set_pair_gain_arm(g); rusty_zstd::set_pair_hi_arm(g); }
                None => { rusty_zstd::set_pair_gain_arm(0.20); rusty_zstd::set_pair_hi_arm(1.0); }
            }
            rusty_zstd::set_pipe_arm(p);
        };
        set(true);  let a=rusty_zstd::compress(src,1).unwrap().len();
        set(true);  let a2=rusty_zstd::compress(src,1).unwrap().len();
        set(false); let b=rusty_zstd::compress(src,1).unwrap().len();
        if a!=a2 { println!("  {id:<10} NONDETERMINISTIC {a} vs {a2}"); }
        if a!=b { moved+=1; println!("  {id:<10} pipe-ON {a:>9} pipe-OFF {b:>9} {:+.3}%",
                    100.0*(b as f64-a as f64)/a as f64); }
    }
    println!("{label}: {moved}/{} diverge\n", IDS.len());
}
fn main(){
    run("route PINNED to PAIR (0.0)", Some(0.0));
    run("route PINNED to OFF (99)",   Some(99.0));
    run("route ADAPTIVE (default)",   None);
    rusty_zstd::set_pipe_arm(true);
}
