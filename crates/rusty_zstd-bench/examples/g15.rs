//! GATE 15 @ L3. Step 1: the CPU dispatch is not dead (AVX2 is present), but is
//! it RIGHT for L3's short matches? All three arms must agree byte-for-byte;
//! only the clock should differ.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],arm:u8,lvl:i32,r:usize)->f64{
    rusty_zstd::set_eqlen_arm(arm);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:u8,b:u8,lvl:i32,r:usize)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,lvl,r); let b1=ms(src,b,lvl,r);
        let b2=ms(src,b,lvl,r); let a2=ms(src,a,lvl,r);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    // correctness first: all arms identical
    let mut bad=0;
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_eqlen_arm(0);
        let a=rusty_zstd::compress(src,lvl).unwrap();
        for arm in [1u8,2]{
            rusty_zstd::set_eqlen_arm(arm);
            if rusty_zstd::compress(src,lvl).unwrap()!=a {bad+=1; println!("MISMATCH {id} arm {arm}");}
        }
        rusty_zstd::set_eqlen_arm(0);
    }
    println!("byte-identity across all three arms at L{lvl}: {bad} mismatches\n");
    println!("{:<12}{:>9}{:>11}{:>11}","corpus","null","words","peek8");
    let (mut tn,mut tw,mut tp,mut k)=(0.0,0.0,0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,0,0,lvl,5); let w=paired(src,0,1,lvl,5); let p=paired(src,0,2,lvl,5);
        println!("{id:<12}{n:>8.2}%{w:>10.2}%{p:>10.2}%");
        tn+=n.abs(); tw+=w; tp+=p; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   words {:+.2}%   peek8 {:+.2}%", tn/k, tw/k, tp/k);
    rusty_zstd::set_eqlen_arm(0);
}
