//! Does the Fast TAG ARRAY earn its per-probe store? It is a SECOND array, so
//! every probe writes two cache lines. Gate 7 is byte-identical, so this is a
//! pure speed question. NULL ARM FIRST -- a verdict below it is not a verdict.
//!   A  no tags array at all   (no store, no filter, smaller footprint)
//!   B  array + filter OFF     (pays the store, gets nothing)
//!   C  array + filter ON      (today's default)
const IDS: &[&str] = &["mr","dickens","sao","ooffice","mozilla","samba","osdb","webster","reymont","smallmsg-8m","x-ray","nci"];
#[derive(Clone,Copy)] enum A { NoArr, ArrOff, ArrOn }
fn set(a:A){ match a {
    A::NoArr  => { rusty_zstd::set_tag_alloc_arm(false); rusty_zstd::set_tag_arm(false); }
    A::ArrOff => { rusty_zstd::set_tag_alloc_arm(true);  rusty_zstd::set_tag_arm(false); }
    A::ArrOn  => { rusty_zstd::set_tag_alloc_arm(true);  rusty_zstd::set_tag_arm(true); }
}}
fn ms(src:&[u8],a:A,n:usize)->f64{ set(a); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn paired(src:&[u8],a:A,b:A)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("{:<14}{:>10}{:>12}{:>12}{:>10}", "corpus","NULL","store cost","filter val","net");
    println!("  vs arm C (today).  positive = today is SLOWER than the alternative\n");
    let (mut tn,mut ts,mut tf,mut tnet,mut n)=(0.0,0.0,0.0,0.0,0.0);
    let mut bad=0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        set(A::ArrOn);  let z1=rusty_zstd::compress(src,1).unwrap();
        set(A::NoArr);  let z2=rusty_zstd::compress(src,1).unwrap();
        if z1!=z2 { bad+=1; }
        assert_eq!(rusty_zstd::decompress(&z2).unwrap(),src,"{id} round-trip");
        let null = paired(src,A::ArrOn,A::ArrOn);
        let store= paired(src,A::ArrOff,A::NoArr);   // array present but unread vs absent
        let filt = paired(src,A::ArrOn,A::ArrOff);   // filter on vs off, array present
        let net  = paired(src,A::ArrOn,A::NoArr);    // today vs no tag machinery at all
        println!("{id:<14}{null:>9.2}%{store:>11.2}%{filt:>11.2}%{net:>9.2}%");
        tn+=null.abs(); ts+=store; tf+=filt; tnet+=net; n+=1.0;
    }
    println!("\nbyte divergences (tags on vs off): {bad}/12  (Gate 7 claims byte-identical)");
    println!("mean |null| {:.2}%  |  store cost {:+.2}%  filter value {:+.2}%  NET today vs none {:+.2}%",
        tn/n, ts/n, tf/n, tnet/n);
    set(A::ArrOn);
}
