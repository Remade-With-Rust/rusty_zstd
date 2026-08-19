//! GATE 14 @ L3: the signal, measured in the band the raise ACTUALLY OPENS.
//! Raising the next-long cut only enables hits where best_ml >= 8. The probe
//! commits at ip+1, spending one literal; what it buys is `ml - best_ml`. So the
//! exchange rate is bytes GAINED per literal SPENT, in that band only.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let g:usize=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(24);
    println!("raising the next-long cut 8 -> {g}\n");
    println!("{:<14}{:>10}{:>11}{:>11}{:>11}{:>10}","corpus","size %","band hits","gain/hit","old ml","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(0);
        let a=rusty_zstd::compress(src,3).unwrap().len() as i64;
        // measure the band WITH the raise on, since that is when those hits occur
        rusty_zstd::set_dfast_good_ml_arm(g);
        let _=rusty_zstd::take_nl_band();
        let b=rusty_zstd::compress(src,3).unwrap().len() as i64;
        let (h,gain,old)=rusty_zstd::take_nl_band();
        let sd=100.0*(b-a) as f64/a as f64;
        rows.push((*id,sd,h,
            if h>0 {gain as f64/h as f64} else {f64::NAN},
            if h>0 {old as f64/h as f64} else {f64::NAN}));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,h,gp,om) in &rows{
        let v=if *sd>0.05 {"LOSES"} else if *sd< -0.02 {"wins"} else {""};
        println!("{id:<14}{sd:>+9.3}%{h:>11}{gp:>11.2}{om:>11.2}{:>10}",v);
    }
    rusty_zstd::set_dfast_good_ml_arm(0);
}
