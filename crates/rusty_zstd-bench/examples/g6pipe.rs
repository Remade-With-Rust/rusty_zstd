//! Gate 6 pays TWICE: the extra probe, AND the loss of the pipelined loop
//! (`if PIPE && !pair`). How much of the cost is the pipeline it gives up?
const IDS: &[&str] = &["mr","dickens","webster","ooffice","smallmsg-8m","reymont","osdb","mozilla","samba","xml","nci"];
fn ms(src: &[u8], pipe: bool, pair: bool, n: usize) -> f64 {
    rusty_zstd::set_pipe_arm(pipe);
    rusty_zstd::set_pair_on_arm(pair);
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let _ = rusty_zstd::compress(src, 1).unwrap();
        let e = t.elapsed().as_secs_f64()*1000.0; if e<b {b=e;}
    }
    b
}
fn paired(src: &[u8], p1: (bool,bool), p2: (bool,bool)) -> f64 {
    let mut d=vec![];
    for _ in 0..3 {
        let a1=ms(src,p1.0,p1.1,5); let b1=ms(src,p2.0,p2.1,5);
        let b2=ms(src,p2.0,p2.1,5); let a2=ms(src,p1.0,p1.1,5);
        d.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
    }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1]
}
fn main() {
    rusty_zstd::set_pair_gain_arm(0.0);
    println!("{:<12}{:>14}{:>16}", "corpus", "PIPE worth", "pair cost(nopipe)");
    println!("  col1 = pair OFF: pipe-off vs pipe-on  (what the pipeline buys)");
    println!("  col2 = pipe OFF both arms: pair-on vs pair-off (probe cost alone)\n");
    let (mut a,mut b,mut n)=(0.0,0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8*1024*1024)];
        let pw  = paired(src,(true,false),(false,false));
        let pc  = paired(src,(false,false),(false,true));
        println!("{id:<12}{pw:>13.2}%{pc:>15.2}%");
        a+=pw; b+=pc; n+=1.0;
    }
    println!("\nmean: pipeline worth {:+.2}% | pair probe alone {:+.2}%", a/n, b/n);
}
