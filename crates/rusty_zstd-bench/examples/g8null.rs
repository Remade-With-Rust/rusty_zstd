//! NULL ARM at L3: the same harness, both arms IDENTICAL (pipe on vs pipe on).
//! Whatever this reports is pure instrument error. Any Gate 8 verdict smaller
//! than it is noise, not a result.
const IDS: &[&str] = &["ooffice","osdb","mr","dickens","sao","mozilla","nci","samba","xml","versions-16m","incomp-32m","x-ray"];
fn ms(src:&[u8],on:bool,n:usize)->f64{ rusty_zstd::set_dfast_pipe_arm(on); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn run(label:&str, arm_b: bool) -> f64 {
    let mut worst=0.0f64; let (mut sum,mut n)=(0.0,0.0);
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
    }
    println!("{label}: mean {:+.2}%  worst |{:.2}%|", sum/n, worst);
    worst
}
fn main(){
    let null = run("NULL ARM   (pipe ON vs pipe ON)", true);
    let real = run("REAL ARM   (pipe ON vs pipe OFF)", false);
    println!("\nnull-arm worst error {:.2}% -- a Gate 8 verdict must EXCEED this to mean anything", null);
    let _ = real;
}
