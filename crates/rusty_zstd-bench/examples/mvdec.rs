//! AVX2 multiversioned sequence loop: does it move DECODE time?
//! Decode is more uniform than encode, so the clock has a better chance here.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(z:&[u8],on:bool,r:usize)->f64{
    rusty_zstd::set_seqmv_arm(on);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::decompress(z).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(z:&[u8],a:bool,b:bool)->f64{
    let mut d=vec![];
    for _ in 0..7 { let a1=ms(z,a,9); let b1=ms(z,b,9); let b2=ms(z,b,9); let a2=ms(z,a,9);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[3]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("DECODE, L{lvl} streams. negative = AVX2 multiversion is FASTER\n");
    println!("{:<12}{:>9}{:>11}{:>13}","corpus","null","avx2 mv","32B copies");
    let (mut tn,mut ta,mut k)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(4<<20)];
        let z=rusty_zstd::compress(src,lvl).unwrap();
        let _=rusty_zstd::take_dec_copies();
        let _=rusty_zstd::decompress(&z).unwrap();
        let (_l32,m32,_l16,_m16)=rusty_zstd::take_dec_copies();
        let n=paired(&z,false,false); let a=paired(&z,false,true);
        println!("{id:<12}{n:>8.2}%{a:>10.2}%{m32:>13}");
        tn+=n.abs(); ta+=a; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean avx2 multiversion {:+.2}%", tn/k, ta/k);
    rusty_zstd::set_seqmv_arm(true);
}
