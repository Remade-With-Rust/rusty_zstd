//! Does the SPECULATION YIELD predict where the DFast pipeline pays?
//! Pair it against the measured time delta per corpus.
const IDS: &[&str] = &["ooffice","osdb","text-32m","mr","dickens","webster","sao","mozilla","zeros-32m","reymont","jsonlog-16m","nci","smallmsg-8m","samba","xml","versions-16m","incomp-32m","x-ray"];
fn ms(src:&[u8],on:bool,n:usize)->f64{ rusty_zstd::set_dfast_pipe_arm(on); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn main(){
    println!("{:<14}{:>10}{:>12}", "corpus", "spec use%", "time %");
    println!("{}","-".repeat(36));
    let mut rows=vec![];
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_dfast_pipe_arm(true);
        let _=rusty_zstd::take_dfast_spec();
        let _=rusty_zstd::compress(src,3).unwrap();
        let (made,used)=rusty_zstd::take_dfast_spec();
        let y=100.0*used as f64/made.max(1) as f64;
        let mut d=vec![];
        for _ in 0..3 {
            let a1=ms(src,false,5); let b1=ms(src,true,5);
            let b2=ms(src,true,5);  let a2=ms(src,false,5);
            d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2));
        }
        d.sort_by(|x,y| x.partial_cmp(y).unwrap());
        println!("{id:<14}{y:>9.1}%{:>11.2}%", d[1]);
        rows.push((*id,y,d[1]));
    }
    rows.sort_by(|a,b| a.1.partial_cmp(&b.1).unwrap());
    println!("\nsorted by speculation use (does it predict the sign?)");
    for (id,y,t) in &rows { println!("  {y:>6.1}% used  {t:>7.2}% time  {id}"); }
}
