//! The pipelined loop never maintained `rep1`. Fixing it makes the two loops
//! byte-identical -- but the stale behaviour was an accidental STICKY REPCODE.
//! Price both, all 18, at L1. Deterministic sizes; paired timing.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn ms(src:&[u8],on:bool,n:usize)->f64{ rusty_zstd::set_pipe_rep1_arm(on); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn main(){
    println!("{:<14}{:>12}{:>12}{:>10}{:>10}", "corpus","STICKY B","MAINTAIN B","size %","time %");
    println!("{}","-".repeat(58));
    let (mut ta,mut tb)=(0u64,0u64);
    let (mut tsum,mut n)=(0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_pipe_rep1_arm(false);
        let a=rusty_zstd::compress(src,1).unwrap().len();
        rusty_zstd::set_pipe_rep1_arm(true);
        let b=rusty_zstd::compress(src,1).unwrap().len();
        let mut d=vec![];
        for _ in 0..3 {
            let a1=ms(src,false,5); let b1=ms(src,true,5);
            let b2=ms(src,true,5);  let a2=ms(src,false,5);
            d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2));
        }
        d.sort_by(|x,y| x.partial_cmp(y).unwrap());
        println!("{id:<14}{a:>12}{b:>12}{:>9.2}%{:>9.2}%",
            100.0*(b as f64-a as f64)/a as f64, d[1]);
        ta+=a as u64; tb+=b as u64; tsum+=d[1]; n+=1.0;
    }
    println!("\nTOTAL sticky {ta}  maintain {tb}  -> {:+.3}%  | mean time {:+.2}%",
        100.0*(tb as f64-ta as f64)/ta as f64, tsum/n);
    println!("(positive size % = MAINTAIN is bigger, i.e. the fix costs ratio)");
    rusty_zstd::set_pipe_rep1_arm(true);
}
