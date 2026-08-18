//! GATE 8 @ L1: null arm FIRST, then the real arm. A verdict smaller than the
//! null arm is not a verdict. Route PINNED -- under Gate 6's adaptive route
//! `set_pipe_arm` is not a byte-identical A/B (the two loops leave equivalent
//! but not identical tables, and the EWMA amplifies it).
const IDS: &[&str] = &["mr","dickens","sao","ooffice","mozilla","samba","x-ray","incomp-32m","osdb","webster"];
fn ms(src:&[u8],on:bool,n:usize)->f64{
    rusty_zstd::set_pipe_arm(on);
    let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn run(label:&str, arm_b: bool){
    // pin the route so the A/B is clean
    rusty_zstd::set_pair_on_arm(true);
    rusty_zstd::set_pair_gain_arm(99.0);   // route 0 everywhere -> pair off -> pipeline eligible
    let (mut sum,mut n,mut worst)=(0.0,0.0,0.0f64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let mut d=vec![];
        for _ in 0..3 {
            let a1=ms(src,true,5); let b1=ms(src,arm_b,5);
            let b2=ms(src,arm_b,5); let a2=ms(src,true,5);
            d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2));
        }
        d.sort_by(|x,y| x.partial_cmp(y).unwrap());
        if d[1].abs()>worst { worst=d[1].abs(); }
        sum+=d[1]; n+=1.0;
        if !arm_b { println!("    {id:<12}{:>8.2}%", d[1]); }
    }
    println!("{label}: mean {:+.2}%  worst |{:.2}%|\n", sum/n, worst);
}
fn main(){
    run("NULL ARM (pipe ON vs pipe ON)", true);
    println!("  per-corpus, pipe OFF vs ON (positive = OFF slower = pipeline WINS):");
    run("REAL ARM (pipe ON vs pipe OFF)", false);
    rusty_zstd::set_pipe_arm(true);
    rusty_zstd::set_pair_gain_arm(0.20);
}
