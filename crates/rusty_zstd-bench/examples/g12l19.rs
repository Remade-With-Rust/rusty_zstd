//! GATE 12 @ L19, part 1: what did the per-jump `std::env::var` cost?
//! text-32m and versions-16m hold 93% of all jumped positions.
const IDS:&[&str]=&["text-32m","versions-16m","zeros-32m","dickens","samba","nci","xml","x-ray","mozilla"];
fn ms(src:&[u8],h:bool,lvl:i32,r:usize)->f64{
    rusty_zstd::set_opt_hoist_arm(h);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:bool,b:bool,lvl:i32,r:usize)->f64{
    let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,lvl,r); let b1=ms(src,b,lvl,r);
        let b2=ms(src,b,lvl,r); let a2=ms(src,a,lvl,r);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[1]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    println!("L{lvl}: env lookup per jumped position -> hoisted to per block");
    println!("negative = the hoist is FASTER\n");
    println!("{:<14}{:>9}{:>11}","corpus","null","hoist");
    let (mut tn,mut tf,mut k)=(0.0,0.0,0.0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let n=paired(src,false,false,lvl,3);
        let f=paired(src,false,true,lvl,3);
        println!("{id:<14}{n:>8.2}%{f:>10.2}%");
        tn+=n.abs(); tf+=f; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean hoist {:+.2}%", tn/k, tf/k);
    rusty_zstd::set_opt_hoist_arm(true);
}
