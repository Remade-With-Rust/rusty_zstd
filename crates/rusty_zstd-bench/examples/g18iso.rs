//! Isolate step 2 from pair_route. 4.69 measured tlen=1, which sets the step AND
//! forces pair_route = 2. If the "free win" was the ROUTE and not the STEP, the
//! dispatch premise is void.
const IDS:&[&str]=&["sao","mr","dickens","ooffice","samba","mozilla","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("L1: step 2 with pair_route UNCHANGED, vs tlen=1 which also flips the route\n");
    println!("{:<12}{:>16}{:>18}{:>12}","corpus","step 2 alone","tlen=1 (step+route)","difference");
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        // baseline: pinned step 1, route as dispatched
        rusty_zstd::set_step_probe_arm(false);
        let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
        // step 2 only: probe latches to 2 (thresholds wide open), route untouched
        rusty_zstd::set_step_probe_arm(true);
        // This used to open TWO gates wide (`step_seq` and `step_forfeit`). The
        // `step_seq` knob has since been removed -- it stored into a static with no
        // reader, so opening it was already a no-op when this board was written.
        rusty_zstd::set_step_forfeit_arm(99.0);
        let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
        // tlen=1: step 2 AND pair_route forced to 2
        rusty_zstd::set_step_probe_arm(false);
        let mut p=rusty_zstd::compression_params(1,Some(src.len() as u64)).unwrap();
        p.target_length=1;
        let c=rusty_zstd::compress_with_params(src,p,false).unwrap().len() as i64;
        let s2=100.0*(b-a) as f64/a as f64;
        let tl=100.0*(c-a) as f64/a as f64;
        println!("{id:<12}{s2:>15.3}%{tl:>17.3}%{:>11.3}%", s2-tl);
    }
    rusty_zstd::set_step_forfeit_arm(0.50);
}
