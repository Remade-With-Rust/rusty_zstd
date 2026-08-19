//! GATE 14 @ L3: does GATE 6's own next-long YIELD separate the corpora where
//! raising the next-long cut wins from the two where it loses?
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("{:<14}{:>11}{:>11}{:>13}{:>10}","corpus","size % @24","nl yield","gain/hit","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
        let _=rusty_zstd::take_next_long();
        let a=rusty_zstd::compress(src,3).unwrap().len() as i64;
        let (p,h,g)=rusty_zstd::take_next_long();
        rusty_zstd::set_dfast_good_ml_arm(24);
        let b=rusty_zstd::compress(src,3).unwrap().len() as i64;
        let sd=100.0*(b-a) as f64/a as f64;
        rows.push((*id,sd,if p>0 {h as f64/p as f64} else {f64::NAN}, if h>0 {g as f64/h as f64} else {f64::NAN}));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,y,gp) in &rows{
        let v=if *sd>0.05 {"LOSES"} else if *sd< -0.02 {"wins"} else {""};
        println!("{id:<14}{sd:>+10.3}%{y:>11.4}{gp:>13.2}{:>10}",v);
    }
    rusty_zstd::set_dfast_good_ml_arm(0);
}
