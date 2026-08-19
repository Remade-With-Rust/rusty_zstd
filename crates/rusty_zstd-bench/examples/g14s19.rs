//! GATE 14 @ L19 step 2: per-corpus outcomes. Win AND loss, or all one way?
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    let d:usize=std::env::args().nth(2).and_then(|s|s.parse().ok()).unwrap_or(16);
    println!("depth 32 -> {d} at L{lvl}\n");
    println!("{:<14}{:>10}{:>11}{:>14}{:>12}","corpus","size %","probe %","probes saved","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_bt_depth_target_arm(0);
        let _=rusty_zstd::take_bt_probe_stats();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pa=rusty_zstd::take_bt_probe_stats().0;
        rusty_zstd::set_bt_depth_target_arm(d);
        let _=rusty_zstd::take_bt_probe_stats();
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pb=rusty_zstd::take_bt_probe_stats().0;
        if pa==0 {continue;}
        rows.push((*id,100.0*(b-a) as f64/a as f64,
                   100.0*(pb as f64-pa as f64)/pa as f64, pa as i64-pb as i64));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,pd,sv) in &rows{
        let v=if *sd>0.10 {"costs size"} else if *sd< -0.001 {"SMALLER"} else {"~free"};
        println!("{id:<14}{sd:>+9.3}%{pd:>+10.2}%{sv:>14}  {v}");
    }
    rusty_zstd::set_bt_depth_target_arm(0);
}
