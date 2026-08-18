//! GATE 9 capability at L3: DFast probe density. Size AND time, paired, with the
//! null arm reported first so a verdict smaller than it is not claimed.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn ms(src:&[u8],st:usize,n:usize)->f64{ rusty_zstd::set_dfast_step_arm(st); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn paired(src:&[u8],a:usize,b:usize)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("{:<14}{:>9}{:>10}{:>10}{:>10}", "corpus","null","sz step2","t step2","t step3");
    let (mut tn,mut ts,mut tt,mut t3,mut n)=(0.0,0.0,0.0,0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_dfast_step_arm(1);
        let s1=rusty_zstd::compress(src,3).unwrap().len() as f64;
        rusty_zstd::set_dfast_step_arm(2);
        let z=rusty_zstd::compress(src,3).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} round-trip");
        let s2=z.len() as f64;
        let null=paired(src,1,1);
        let d2=paired(src,1,2);
        let d3=paired(src,1,3);
        println!("{id:<14}{null:>8.2}%{:>9.2}%{d2:>9.2}%{d3:>9.2}%", 100.0*(s2-s1)/s1);
        tn+=null.abs(); ts+=100.0*(s2-s1)/s1; tt+=d2; t3+=d3; n+=1.0;
    }
    println!("\nmean |null| {:.2}%  |  step2: size {:+.2}%  time {:+.2}%  |  step3 time {:+.2}%",
        tn/n, ts/n, tt/n, t3/n);
    rusty_zstd::set_dfast_step_arm(1);
}
