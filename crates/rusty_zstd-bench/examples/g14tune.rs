//! GATE 14 @ L3: sweep the offset-trade threshold. mr still pays for the warm-up
//! and the re-probes -- how much of that is threshold and how much is schedule?
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let mut base=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(8);
        rusty_zstd::set_nl_off_worse_arm(-1.0);
        base.push((id.to_string(), rusty_zstd::compress(src,lvl).unwrap().len() as i64));
    }
    let bt:i64=base.iter().map(|x|x.1).sum();
    println!("{:>7}{:>13}{:>11}{:>10}{:>10}   worst","thresh","size","size %","mr","osdb");
    for t in [2.0f32,0.80,0.75,0.70,0.65,0.60,0.50,0.40]{
        rusty_zstd::set_dfast_good_ml_arm(24);
        rusty_zstd::set_nl_off_worse_arm(t);
        let mut tot=0i64; let (mut mr,mut osdb)=(0.0,0.0);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            tot+=z;
            let d=100.0*(z-base[k].1) as f64/base[k].1 as f64;
            if *id=="mr" {mr=d;} if *id=="osdb" {osdb=d;}
            if d>w {w=d; wid=id;}
        }
        let lbl=if t>1.0 {"forced".to_string()} else {format!("{t:.2}")};
        println!("{lbl:>7}{tot:>13}{:>+10.4}%{mr:>+9.3}%{osdb:>+9.3}%   {wid} {w:+.3}%",
            100.0*(tot-bt) as f64/bt as f64);
    }
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_nl_off_worse_arm(0.70);
}
