//! GATE 14 @ L3: the probe takes a DIFFERENT match at a DIFFERENT OFFSET.
//! Offset bits are what the extra match bytes must pay for. Does the offset
//! change separate the two losers?
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let g:usize=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(24);
    println!("next-long cut 8 -> {g}: what the raised-band hits actually trade\n");
    println!("{:<14}{:>9}{:>9}{:>12}{:>12}{:>9}{:>9}","corpus","size %","gain/hit","off new","off old","worse %","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(0);
        let a=rusty_zstd::compress(src,3).unwrap().len() as i64;
        rusty_zstd::set_dfast_good_ml_arm(g);
        let _=rusty_zstd::take_nl_band(); let _=rusty_zstd::take_nl_off();
        let b=rusty_zstd::compress(src,3).unwrap().len() as i64;
        let (h,gain,_old)=rusty_zstd::take_nl_band();
        let (on,oo,worse)=rusty_zstd::take_nl_off();
        if h==0 {continue;}
        rows.push((*id, 100.0*(b-a) as f64/a as f64, gain as f64/h as f64,
                   on as f64/h as f64, oo as f64/h as f64, 100.0*worse as f64/h as f64));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,gp,on,oo,w) in &rows{
        let v=if *sd>0.05 {"LOSES"} else if *sd< -0.02 {"wins"} else {""};
        println!("{id:<14}{sd:>+8.3}%{gp:>9.2}{on:>12.0}{oo:>12.0}{w:>8.1}%{:>9}",v);
    }
    rusty_zstd::set_dfast_good_ml_arm(0);
}
