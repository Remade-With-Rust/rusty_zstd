//! GATE 14 @ L19: does the NO-GAIN probe share separate the corpora where the
//! depth cut is free from nci, where it costs 0.257%? The counter already
//! exists -- BT_NOGAIN, "75.7% no-gain probes at L19" in the shipped comment.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","versions-16m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("{:<14}{:>10}{:>10}{:>11}{:>10}{:>11}","corpus","size%@24","probe%@24","no-gain %","short %","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_bt_depth_target_arm(0);
        let _=rusty_zstd::take_bt_probe_stats();
        let a=rusty_zstd::compress(src,19).unwrap().len() as i64;
        let (pa,sh,ng)=rusty_zstd::take_bt_probe_stats();
        rusty_zstd::set_bt_depth_target_arm(24);
        let _=rusty_zstd::take_bt_probe_stats();
        let b=rusty_zstd::compress(src,19).unwrap().len() as i64;
        let pb=rusty_zstd::take_bt_probe_stats().0;
        if pa==0 {continue;}
        rows.push((*id,100.0*(b-a) as f64/a as f64,
            100.0*(pb as f64-pa as f64)/pa as f64,
            100.0*ng as f64/pa as f64, 100.0*sh as f64/pa as f64));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,pd,ng,sh) in &rows{
        let v=if *sd>0.10 {"COSTS"} else if *pd< -3.0 {"free win"} else {""};
        println!("{id:<14}{sd:>+9.3}%{pd:>+9.2}%{ng:>10.1}%{sh:>9.1}%{:>11}",v);
    }
    rusty_zstd::set_bt_depth_target_arm(0);
}
