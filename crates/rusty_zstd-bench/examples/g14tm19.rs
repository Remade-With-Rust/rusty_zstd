//! GATE 14 @ L19 dispatch on the CLOCK. Movers and non-movers both, so the
//! non-movers act as a live null arm.
const IDS:&[&str]=&["mr","samba","mozilla","nci","webster","xml","dickens","osdb"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],on:bool,r:usize)->f64{
    rusty_zstd::set_bt_deep_min_arm(if on {2.0} else {f32::MAX});
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,19).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:bool,b:bool)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,4); let b1=ms(src,b,4); let b2=ms(src,b,4); let a2=ms(src,a,4);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    println!("L19, 2 MiB, best-of-4 x ABBA x5, median. negative = FASTER\n");
    println!("{:<12}{:>9}{:>11}{:>12}","corpus","null","dispatch","expected");
    let (mut tn,mut td,mut k)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let n=paired(src,false,false); let d=paired(src,false,true);
        let exp=match *id {"mr"=>"cut","samba"=>"cut","mozilla"=>"cut",_=>"unchanged"};
        println!("{id:<12}{n:>8.2}%{d:>10.2}%{exp:>12}");
        tn+=n.abs(); td+=d; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean dispatch {:+.2}%", tn/k, td/k);
    rusty_zstd::set_bt_deep_min_arm(2.0);
}
