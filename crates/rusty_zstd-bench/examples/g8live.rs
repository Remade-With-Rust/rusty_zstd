//! GATE 8 protocol step 1: is the gate DEAD at this level? A gate that is
//! byte-identical by construction reads 0/18 whether it ran or not, so size
//! alone cannot answer it -- pair it with the finder call counts.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn ms(src:&[u8],lvl:i32,pipe:bool,n:usize)->f64{
    rusty_zstd::set_pipe_arm(pipe);
    let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn main(){
    for &lvl in &[3i32,1] {
        let cap = 8usize<<20;
        let (mut moved, mut fast_calls, mut dfast_calls, mut tsum, mut n)=(0,0u64,0u64,0.0,0.0);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(cap)];
            let _=rusty_zstd::take_finder_calls(); let _=rusty_zstd::take_dfast_calls();
            rusty_zstd::set_pipe_arm(true);
            let a=rusty_zstd::compress(src,lvl).unwrap().len();
            let (fc,_)=rusty_zstd::take_finder_calls();
            let (ds,dr)=rusty_zstd::take_dfast_calls();
            rusty_zstd::set_pipe_arm(false);
            let b=rusty_zstd::compress(src,lvl).unwrap().len();
            if a!=b { moved+=1; }
            fast_calls+=fc; dfast_calls+=ds+dr;
            // paired timing: pipe ON vs OFF
            let mut d=vec![];
            for _ in 0..3 {
                let a1=ms(src,lvl,false,5); let b1=ms(src,lvl,true,5);
                let b2=ms(src,lvl,true,5);  let a2=ms(src,lvl,false,5);
                d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2));
            }
            d.sort_by(|x,y| x.partial_cmp(y).unwrap());
            tsum+=d[1]; n+=1.0;
        }
        println!("L{lvl}: {moved}/18 sizes move | find_fast calls {fast_calls} | find_dfast calls {dfast_calls}");
        println!("      pipe ON vs OFF, paired: {:+.2}%  (negative = pipe faster)\n", tsum/n);
        rusty_zstd::set_pipe_arm(true);
    }
}
