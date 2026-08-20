//! AUDIT: is pair_gain / pair_route a dispatch axis for the OTHER L1 gates that
//! were ruled CONSTANT? For each arm, per-corpus effect on SIZE and WORK, printed
//! next to pair_gain and route2%, so a sign-flip can be checked against them.
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn meas(src:&[u8])->(i64,u64,f64,f64){
    let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_route_hist();
    let z=rusty_zstd::compress(src,1).unwrap();
    let pos=rusty_zstd::take_mm().0;
    let (r0,r1,r2,g,_)=rusty_zstd::take_route_hist();
    (z.len() as i64,pos,g,100.0*r2 as f64/(r0+r1+r2).max(1) as f64)
}
fn sweep(name:&str, on:&dyn Fn(), off:&dyn Fn()){
    println!("\n===== {name} =====");
    println!("{:<14}{:>10}{:>10}{:>11}{:>11}","corpus","pair_gain","route2 %","d size","d positions");
    let mut rows=vec![];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        on(); let (a,pa,g,r2)=meas(src);
        off(); let (b,pb,_,_)=meas(src);
        on();
        let ds=100.0*(b-a) as f64/a as f64;
        let dp=100.0*(pb as f64-pa as f64)/pa.max(1) as f64;
        if ds.abs()<1e-9 && dp.abs()<1e-9 {continue;}
        rows.push((id.to_string(),g,r2,ds,dp));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,g,r2,ds,dp) in &rows{
        println!("{id:<14}{g:>10.4}{r2:>9.1}%{ds:>+10.4}%{dp:>+10.2}%");
    }
    if rows.is_empty(){println!("  (arm moves nothing at L1)");}
    else{
        let neg:Vec<_>=rows.iter().filter(|r|r.3< -1e-9).map(|r|r.0.clone()).collect();
        let pos:Vec<_>=rows.iter().filter(|r|r.3>1e-9).map(|r|r.0.clone()).collect();
        println!("  size SIGN-FLIP: {} smaller / {} larger -> {}",
            neg.len(),pos.len(),
            if !neg.is_empty() && !pos.is_empty() {"YES (dispatch candidate)"} else {"no"});
    }
}
fn main(){
    println!("AUDIT: pair_gain / pair_route as a dispatch axis for the L1 CONSTANT gates");
    sweep("GATE 13 @ L1 -- literal push tiers (set_litpush_arm)",
        &||rusty_zstd::set_litpush_arm(true), &||rusty_zstd::set_litpush_arm(false));
    sweep("GATE 15 @ L1 -- count_eq_len AVX2 (set_eqlen_arm)",
        &||rusty_zstd::set_eqlen_arm(0), &||rusty_zstd::set_eqlen_arm(1));
    sweep("GATE 16 @ L1 -- skip_search (set_raw_skip_arm)",
        &||rusty_zstd::set_raw_skip_arm(true), &||rusty_zstd::set_raw_skip_arm(false));
    sweep("GATE 18 @ L1 -- the SHIPPED dispatch (set_pair_lo_arm), for reference",
        &||rusty_zstd::set_pair_lo_arm(0.71), &||rusty_zstd::set_pair_lo_arm(0.0));
    rusty_zstd::set_pair_lo_arm(f32::NAN);
}
