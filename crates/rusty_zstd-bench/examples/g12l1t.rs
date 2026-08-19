//! (1) `mr` re-timed at higher rep count -- the defect fix is the only change.
//! (2) GATE 12 @ L1's own question: does dropping a fill buy speed at L1, where
//!     the table is 64x smaller than L7 and the writes are short-table only?
fn ms1(src:&[u8],fixed:bool,r:usize)->f64{ rusty_zstd::set_replen_pipe_arm(fixed);
    let mut b=f64::MAX; for _ in 0..r { let t=std::time::Instant::now();
        let _=rusty_zstd::compress(src,1).unwrap(); let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn ms2(src:&[u8],n:u8,r:usize)->f64{ rusty_zstd::set_dfast_fill_n_arm(n);
    let mut b=f64::MAX; for _ in 0..r { let t=std::time::Instant::now();
        let _=rusty_zstd::compress(src,1).unwrap(); let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn p1(src:&[u8],a:bool,b:bool,r:usize)->f64{ let mut d=vec![];
    for _ in 0..5 { let a1=ms1(src,a,r); let b1=ms1(src,b,r); let b2=ms1(src,b,r); let a2=ms1(src,a,r);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2] }
fn p2(src:&[u8],a:u8,b:u8,r:usize)->f64{ let mut d=vec![];
    for _ in 0..5 { let a1=ms2(src,a,r); let b1=ms2(src,b,r); let b2=ms2(src,b,r); let a2=ms2(src,a,r);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2] }
fn main(){
    let Ok(full)=std::fs::read("corpora/data/silesia/mr") else{return};
    let src=&full[..full.len().min(8<<20)];
    println!("(1) rep_len_ratio defect fix, `mr`, best-of-9 x ABBA x5, median:");
    println!("      null {:>7.2}%      fix {:>7.2}%\n", p1(src,false,false,9), p1(src,false,true,9));
    rusty_zstd::set_replen_pipe_arm(true);
    println!("(2) GATE 12 @ L1 -- dropping a fill (work -20.37%, size +0.1538%):");
    println!("{:<12}{:>9}{:>12}{:>12}","corpus","null","drop end-2","drop both");
    for id in ["mr","dickens","samba","x-ray","mozilla","sao"]{
        let Ok(f)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let s=&f[..f.len().min(8<<20)];
        println!("{id:<12}{:>8.2}%{:>11.2}%{:>11.2}%", p2(s,2,2,7), p2(s,2,1,7), p2(s,2,0,7));
    }
    rusty_zstd::set_dfast_fill_n_arm(2);
}
