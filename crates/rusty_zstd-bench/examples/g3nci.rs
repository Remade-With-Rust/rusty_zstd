//! The impossible number: gating 81.5% of nci's fill inserts costs MORE size
//! than removing the fill entirely. Raw bytes for every arm.
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(7);
    for id in ["nci","xml","versions-16m"]{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        println!("\n=== {id} @ L{lvl} ===");
        let mut base=0i64;
        for (label,on,thr) in [("fill OFF        ",false,0.0f32),("fill ON  thr 0.0",true,0.0),
                               ("fill ON  thr .05",true,0.05),("fill ON  thr 0.1",true,0.1),
                               ("fill ON  thr 0.2",true,0.2)]{
            rusty_zstd::set_lazy_fill_arm(on);
            rusty_zstd::set_lazy_fill_threshold_arm(thr);
            let _=rusty_zstd::take_lazy_fill();
            let sz=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            let (f,ne,ins)=rusty_zstd::take_lazy_fill();
            if base==0 {base=sz;}
            println!("{label}  size {sz:>9}  {:>+9} ({:>+7.3}%)  fills {f:>9} nonempty {ne:>9} inserts {ins:>9}",
                sz-base, 100.0*(sz-base) as f64/base as f64);
        }
        rusty_zstd::set_lazy_fill_arm(true);
        rusty_zstd::set_lazy_fill_threshold_arm(0.0);
    }
}
