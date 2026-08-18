const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn ms(src:&[u8],st:usize,n:usize)->f64{ rusty_zstd::set_dfast_step_arm(st); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn paired(src:&[u8],a:usize,b:usize)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("{:<14}{:>11}{:>11}", "corpus","size","time");
    let (mut ts,mut tt,mut n,mut worst)=(0.0,0.0,0.0,0.0f64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_dfast_step_arm(1);
        let s1=rusty_zstd::compress(src,3).unwrap().len() as f64;
        rusty_zstd::set_dfast_step_arm(0); // dispatch
        let z=rusty_zstd::compress(src,3).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id}");
        let d=100.0*(z.len() as f64-s1)/s1;
        let t=paired(src,1,0);
        println!("{id:<14}{d:>10.2}%{t:>10.2}%");
        ts+=d; tt+=t; n+=1.0; if d>worst {worst=d;}
    }
    println!("\nDISPATCHED vs step1: size {:+.2}%  time {:+.2}%  worst size regression {:.2}%", ts/n, tt/n, worst);
    rusty_zstd::set_dfast_step_arm(0);
}
