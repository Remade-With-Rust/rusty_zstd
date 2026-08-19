//! GATE 12 @ L3 as a SPEED question. Dropping the `match_end-2` fill removes
//! ~50% of DFast's per-match table writes (7.51M -> 3.76M across the 18) for
//! +0.4519% size. 4.39 priced only the main-loop positions it COSTS (+17,045)
//! and called it dominated; the writes it REMOVES are 220x larger. Null arm first.
const IDS:&[&str]=&["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn ms(src:&[u8],n:u8,r:usize)->f64{
    rusty_zstd::set_dfast_fill_n_arm(n);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:u8,b:u8)->f64{
    let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[1]
}
fn main(){
    println!("{:<14}{:>9}{:>11}{:>11}{:>11}","corpus","null","size drop","time drop","time both");
    println!("  negative time = dropping the end-2 fill is FASTER");
    println!("  'time both'   = dropping BOTH end fills\n");
    let (mut tn,mut ts,mut tt,mut tb,mut k)=(0.0,0.0,0.0,0.0,0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_dfast_fill_n_arm(2);
        let on=rusty_zstd::compress(src,3).unwrap().len() as f64;
        rusty_zstd::set_dfast_fill_n_arm(1);
        let off=rusty_zstd::compress(src,3).unwrap().len() as f64;
        let null=paired(src,2,2);
        let t1=paired(src,2,1);
        let t0=paired(src,2,0);
        let sz=100.0*(off-on)/on;
        println!("{id:<14}{null:>8.2}%{sz:>10.3}%{t1:>10.2}%{t0:>10.2}%");
        tn+=null.abs(); ts+=sz; tt+=t1; tb+=t0; k+=1;
        rusty_zstd::set_dfast_fill_n_arm(2);
    }
    let k=k as f64;
    println!("\nmean |null| {:.2}%   mean size {:+.3}%   mean time drop-end2 {:+.2}%   drop-both {:+.2}%",
        tn/k, ts/k, tt/k, tb/k);
}
