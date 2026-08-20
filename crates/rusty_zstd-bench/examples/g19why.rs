//! WHY does GATE 5 not fire on the corpora with the biggest available wins?
//! L1 Fast ladder: only raw-escape (r_prev >= 0.70) and drift (>= 2.00) are live;
//! match-reach is an OFF switch (rep >= 2.00, and rep_yield <= 1.0).
const IDS:&[(&str,f64)]=&[("versions-16m",-3.935),("mr",-1.023),("xml",-0.432),
 ("reymont",-0.120),("mozilla",-1.233),("samba",-0.380),("sao",-0.001),("x-ray",-0.014)];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("GATE 5 @ L1: thresholds are r_prev >= 0.70 (raw-escape), drift >= 2.00\n");
    println!("{:<14}{:>10}{:>10}{:>9}{:>9}{:>9}","corpus","available","r_prev","drift","reduced","fires?");
    for (id,avail) in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let _=rusty_zstd::take_g5(); let _=rusty_zstd::take_g5_inputs();
        let _=rusty_zstd::compress(src,1).unwrap();
        let (c,r,d)=rusty_zstd::take_g5();
        let (rp,dr)=rusty_zstd::take_g5_inputs();
        let cov=if c>0 {100.0*(r+d) as f64/c as f64} else {0.0};
        let why=if rp>=0.70 {"raw-esc"} else if dr>=2.00 {"drift"} else {"NEITHER"};
        println!("{id:<14}{avail:>+9.3}%{rp:>10.4}{dr:>9.4}{cov:>8.1}%{why:>9}");
    }
}
