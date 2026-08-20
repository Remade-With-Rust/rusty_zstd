//! GATE 18 @ L3: mls=8 cuts BOTH work terms (-5.04% positions, -35.92%
//! sequences) for +3.76% size. Time it paired -- the priority is speed.
const IDS:&[&str]=&["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],m:u32,r:usize)->f64{
    let mut p=rusty_zstd::compression_params(3,Some(src.len() as u64)).unwrap();
    p.min_match=m;
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now();
        let _=rusty_zstd::compress_with_params(src,p,false).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:u32,b:u32)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,5); let b1=ms(src,b,5);
        let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    println!("L3, 8 MiB, best-of-5 x ABBA x5. negative = faster than shipped mls=5\n");
    println!("{:<12}{:>9}{:>10}{:>10}{:>10}","corpus","null","mls=4","mls=6","mls=8");
    let (mut tn,mut t4,mut t6,mut t8,mut k)=(0.0,0.0,0.0,0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,5,5); let a=paired(src,5,4);
        let b=paired(src,5,6); let c=paired(src,5,8);
        println!("{id:<12}{n:>8.2}%{a:>9.2}%{b:>9.2}%{c:>9.2}%");
        tn+=n.abs(); t4+=a; t6+=b; t8+=c; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mls=4 {:+.2}%   mls=6 {:+.2}%   mls=8 {:+.2}%",
        tn/k, t4/k, t6/k, t8/k);
}
