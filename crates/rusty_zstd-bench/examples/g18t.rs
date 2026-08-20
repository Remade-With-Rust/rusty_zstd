//! Does the -15.04% position saving convert to TIME? GATE 14 @ L3 shipped a
//! work-count win that measured +2.5% SLOWER; this is the check that caught it.
const IDS:&[&str]=&["mr","dickens","samba","sao","ooffice","mozilla","webster","nci","xml","osdb"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],on:bool,r:usize)->f64{
    rusty_zstd::set_step_probe_arm(on);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
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
    println!("L1, 8 MiB, best-of-7 x ABBA x5. negative = the dispatch is FASTER\n");
    println!("{:<12}{:>9}{:>11}","corpus","null","dispatch");
    let (mut tn,mut td,mut k)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,false,false); let d=paired(src,false,true);
        println!("{id:<12}{n:>8.2}%{d:>10.2}%");
        tn+=n.abs(); td+=d; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean dispatch {:+.2}%", tn/k, td/k);
    rusty_zstd::set_step_probe_arm(true);
}
