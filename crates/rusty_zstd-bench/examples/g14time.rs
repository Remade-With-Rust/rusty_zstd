//! GATE 14 @ L3: the SPEED delta, measured rather than assumed. Work count says
//! -0.38% probes; the question is whether that resolves on this box at all.
const IDS:&[&str]=&["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn base(){ rusty_zstd::set_dfast_good_ml_arm(8); rusty_zstd::set_dfast_good_ml2_arm(8);
           rusty_zstd::set_nl_off_worse_arm(-1.0); }
fn ship(){ rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
           rusty_zstd::set_nl_off_worse_arm(0.60); }
fn ms(src:&[u8],shipped:bool,r:usize)->f64{
    if shipped {ship()} else {base()}
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:bool,b:bool)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,7); let b1=ms(src,b,7); let b2=ms(src,b,7); let a2=ms(src,a,7);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    println!("L3, 8 MiB, best-of-7 x ABBA x5, median. negative = shipped is FASTER\n");
    println!("{:<12}{:>9}{:>11}","corpus","null","gate 14");
    let (mut tn,mut tg,mut k)=(0.0,0.0,0.0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,false,false); let g=paired(src,false,true);
        println!("{id:<12}{n:>8.2}%{g:>10.2}%");
        tn+=n.abs(); tg+=g; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean gate 14 {:+.2}%", tn/k, tg/k);
    ship();
}
