//! Time the rep_len_ratio defect fix. Only `mr` changes behaviour (-53.1% of its
//! repcode probes); the rest are a null arm by construction -- a free noise check.
const IDS:&[&str]=&["mr","ooffice","sao","dickens","samba","x-ray"];
fn ms(src:&[u8],fixed:bool,r:usize)->f64{
    rusty_zstd::set_replen_pipe_arm(fixed);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:bool,b:bool)->f64{
    let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[1]
}
fn main(){
    println!("{:<12}{:>9}{:>11}","corpus","null","fix");
    println!("  negative = the fix is FASTER. Only `mr` changes behaviour;");
    println!("  the others are a live null arm.\n");
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let null=paired(src,false,false);
        let t=paired(src,false,true);
        let mark=if *id=="mr" {"   <- the only corpus that changes"} else {""};
        println!("{id:<12}{null:>8.2}%{t:>10.2}%{mark}");
    }
    rusty_zstd::set_replen_pipe_arm(true);
}
