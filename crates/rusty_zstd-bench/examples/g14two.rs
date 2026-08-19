//! GATE 14 @ L3: the two sites, swept apart. Which one carries the size loss on
//! mr and osdb, and which one carries the win?
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(g1:usize,g2:usize)->(i64,u64,Vec<(String,i64)>){
    let (mut sz,mut pr)=(0i64,0u64); let mut per=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(g1);
        rusty_zstd::set_dfast_good_ml2_arm(g2);
        let _=rusty_zstd::take_mm();
        let z=rusty_zstd::compress(src,3).unwrap().len() as i64;
        sz+=z; pr+=rusty_zstd::take_mm().0; per.push((id.to_string(),z));
    }
    (sz,pr,per)
}
fn main(){
    let (bs,bp,base)=run(0,0);
    println!("{:>8}{:>8}{:>13}{:>13}{:>11}{:>11}   worst","nextlong","cand2","size","probes","size %","probe %");
    println!("{:>8}{:>8}{:>13}{:>13}{:>11}{:>11}   (shipped)",8,8,bs,bp,"-","-");
    for (g1,g2) in [(24,8),(8,24),(24,24),(16,8),(8,16),(12,32),(32,12)]{
        let (sz,pr,per)=run(g1,g2);
        let (mut w,mut wid)=(0f64,String::new());
        for (k,(id,z)) in per.iter().enumerate(){
            let d=100.0*(z-base[k].1) as f64/base[k].1 as f64;
            if d>w {w=d; wid=id.clone();}
        }
        println!("{g1:>8}{g2:>8}{sz:>13}{pr:>13}{:>+10.4}%{:>+10.2}%   {wid} {w:+.3}%",
            100.0*(sz-bs) as f64/bs as f64, 100.0*(pr as f64-bp as f64)/bp as f64);
    }
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
}
