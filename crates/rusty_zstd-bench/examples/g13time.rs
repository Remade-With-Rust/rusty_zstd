//! GATE 13 extended to DFast: reserve + fixed-width literal copy. Byte-identical,
//! so this is pure speed at zero size. Null arm alongside.
const IDS:&[&str]=&["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn ms(src:&[u8],on:bool,r:usize)->f64{
    rusty_zstd::set_dfast_litpush_arm(on);
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
    println!("L3: negative = GATE 13 on DFast is FASTER (byte-identical)\n");
    println!("{:<12}{:>9}{:>11}","corpus","null","gate 13");
    let (mut tn,mut tg,mut k)=(0.0,0.0,0.0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,false,false); let g=paired(src,false,true);
        println!("{id:<12}{n:>8.2}%{g:>10.2}%");
        tn+=n.abs(); tg+=g; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean gate 13 {:+.2}%", tn/k, tg/k);
    rusty_zstd::set_dfast_litpush_arm(true);
}
