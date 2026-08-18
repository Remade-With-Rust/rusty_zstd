//! Re-price the tag array's per-probe STORE now that the atomics are gone.
//! Packing is only worth its risk if this is still material.
//!   A  array present, filter OFF  (pays the store, gets nothing)
//!   B  no array at all
const IDS: &[&str] = &["mr","dickens","sao","ooffice","mozilla","samba","osdb","webster","reymont","smallmsg-8m","nci","xml"];
fn ms(src:&[u8],alloc:bool,n:usize)->f64{
    rusty_zstd::set_tag_alloc_arm(alloc); rusty_zstd::set_tag_arm(false);
    let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn main(){
    println!("{:<14}{:>10}{:>12}", "corpus","null","store cost");
    let (mut tn,mut ts,mut n)=(0.0,0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let mut nu=vec![]; let mut st=vec![];
        for _ in 0..3 {
            // null: alloc vs alloc
            let a1=ms(src,true,5); let b1=ms(src,true,5);
            let b2=ms(src,true,5); let a2=ms(src,true,5);
            nu.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2));
            // real: alloc vs no-alloc  (positive = removing the array is FASTER)
            let c1=ms(src,true,5); let d1=ms(src,false,5);
            let d2=ms(src,false,5); let c2=ms(src,true,5);
            st.push(0.5*(100.0*(c1-d1)/d1+100.0*(c2-d2)/d2));
        }
        nu.sort_by(|a,b| a.partial_cmp(b).unwrap());
        st.sort_by(|a,b| a.partial_cmp(b).unwrap());
        println!("{id:<14}{:>9.2}%{:>11.2}%", nu[1], st[1]);
        tn+=nu[1].abs(); ts+=st[1]; n+=1.0;
    }
    println!("\nmean |null| {:.2}%   store cost {:+.2}%  (positive = the array's store COSTS this much)", tn/n, ts/n);
    rusty_zstd::set_tag_alloc_arm(true); rusty_zstd::set_tag_arm(true);
}
